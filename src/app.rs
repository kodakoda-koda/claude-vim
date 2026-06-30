use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crate::input::{InputAction, InputMatcher};
use crate::scroll::Scroller;
use crate::pty::PtySession;
use crate::statusline::{Mode, StatuslineConfig, render};

const ESC_TIMEOUT_MS: u64 = 50;
const POLL_INTERVAL_MS: u64 = 5;
const SCROLL_THROTTLE_MS: u64 = 50;

/// アプリケーション全体の状態
pub struct App {
    mode: Mode,
    pty: PtySession,
    scroller: Scroller,
    statusline_config: StatuslineConfig,
    input_matcher: InputMatcher,
    sigwinch_flag: Arc<AtomicBool>,
    /// ターミナル行数（ステータスライン込みの全行）
    rows: u16,
    /// ターミナル列数
    cols: u16,
    /// 最後のスクロールイベント送信時刻（加速防止スロットル用）
    last_scroll: Instant,
}

impl App {
    pub fn new(
        pty: PtySession,
        scroller: Scroller,
        statusline_config: StatuslineConfig,
        sigwinch_flag: Arc<AtomicBool>,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            mode: Mode::Insert,
            pty,
            scroller,
            statusline_config,
            input_matcher: InputMatcher::new(),
            sigwinch_flag,
            rows,
            cols,
            last_scroll: Instant::now(),
        })
    }

    /// イベントループを開始する。cv の起動から終了まで実行し続ける
    pub fn run(mut self) -> anyhow::Result<()> {
        // stdin reader スレッドを起動
        let (stdin_tx, stdin_rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            use std::io::Read;
            let stdin = std::io::stdin();
            let mut handle = stdin.lock();
            let mut buf = [0u8; 256];
            loop {
                match handle.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if stdin_tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let mut stdout = std::io::stdout();

        loop {
            // 1. SIGWINCH チェック
            if self.sigwinch_flag.swap(false, Ordering::Relaxed)
                && let Ok((cols, rows)) = crossterm::terminal::size()
            {
                self.cols = cols;
                self.rows = rows;
                self.statusline_config.cols = cols;
                self.scroller.set_size(cols, rows.saturating_sub(1));
                let _ = self.pty.resize(cols, rows.saturating_sub(1));
                let _ = render(self.mode, &self.statusline_config, rows - 1);
                let _ = stdout.flush();
            }

            // 2. PTY 出力を drain して stdout に書く
            let mut had_output = false;
            while let Ok(bytes) = self.pty.output_rx().try_recv() {
                if !bytes.is_empty() {
                    let _ = stdout.write_all(&bytes);
                    had_output = true;
                }
            }
            if had_output {
                let _ = render(self.mode, &self.statusline_config, self.rows - 1);
                let _ = stdout.flush();
            }

            // 3. stdin から入力を処理する
            // スクロール後は break して PTY 出力を先に処理する（加速防止）
            let mut had_input = false;
            loop {
                match stdin_rx.try_recv() {
                    Ok(bytes) => {
                        had_input = true;
                        let did_scroll = self.handle_input(bytes, &stdin_rx)?;
                        if did_scroll {
                            break;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                }
            }

            // 4. claude プロセス終了チェック
            if self.pty.is_exited() {
                break;
            }

            if !had_output && !had_input {
                std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
            }
        }

        Ok(())
    }

    /// 入力を処理する。スクロールが発生した場合は true を返す
    fn handle_input(
        &mut self,
        bytes: Vec<u8>,
        stdin_rx: &mpsc::Receiver<Vec<u8>>,
    ) -> anyhow::Result<bool> {
        match self.mode {
            Mode::Insert => {
                self.handle_insert_input(bytes, stdin_rx)?;
                Ok(false)
            }
            Mode::Normal => self.handle_normal_input(bytes),
        }
    }

    fn handle_insert_input(
        &mut self,
        bytes: Vec<u8>,
        stdin_rx: &mpsc::Receiver<Vec<u8>>,
    ) -> anyhow::Result<()> {
        // Kitty keyboard protocol level 1 では Esc が \x1b[27u として来る
        if bytes == b"\x1b[27u" {
            self.switch_to_normal()?;
            return Ok(());
        }

        // 従来の Esc 単独検出（後続バイトなし 50ms タイムアウト）
        if bytes == [0x1b] {
            match stdin_rx.recv_timeout(Duration::from_millis(ESC_TIMEOUT_MS)) {
                Ok(following) => {
                    // Kitty kbd protocol: \x1b が分割して来た場合も検出
                    if following == b"[27u" {
                        self.switch_to_normal()?;
                    } else {
                        let mut combined = bytes;
                        combined.extend_from_slice(&following);
                        self.pty.write_bytes(&combined)?;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.switch_to_normal()?;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {}
            }
        } else if bytes == [b'\r'] || bytes == b"\x1b[13u" {
            // Insert mode: Enter → 改行（Shift+Enter として PTY に送る）
            self.pty.write_bytes(b"\x1b[13;2u")?;
        } else {
            self.pty.write_bytes(&bytes)?;
        }
        Ok(())
    }

    /// Normal mode の入力を処理する。スクロールが発生した場合は true を返す
    fn handle_normal_input(&mut self, bytes: Vec<u8>) -> anyhow::Result<bool> {
        let (action, new_matcher) = self.input_matcher.process(&bytes);
        self.input_matcher = new_matcher;

        match action {
            InputAction::Scroll(amount) => {
                let now = Instant::now();
                if now.duration_since(self.last_scroll) < Duration::from_millis(SCROLL_THROTTLE_MS) {
                    return Ok(true);
                }
                let scroll_data = self.scroller.scroll_bytes(amount);
                self.pty.write_bytes(&scroll_data)?;
                self.last_scroll = now;
                Ok(true)
            }
            InputAction::SwitchToInsert => {
                self.switch_to_insert()?;
                Ok(false)
            }
            InputAction::Passthrough(data) => {
                self.pty.write_bytes(&data)?;
                if data == b"g" && bytes != data {
                    let (action2, new_matcher2) = self.input_matcher.process(&bytes);
                    self.input_matcher = new_matcher2;
                    if let InputAction::Scroll(amount) = action2 {
                        let now = Instant::now();
                        if now.duration_since(self.last_scroll) >= Duration::from_millis(SCROLL_THROTTLE_MS) {
                            let scroll_data = self.scroller.scroll_bytes(amount);
                            self.pty.write_bytes(&scroll_data)?;
                            self.last_scroll = now;
                        }
                        return Ok(true);
                    } else if let InputAction::Passthrough(data2) = action2 {
                        self.pty.write_bytes(&data2)?;
                    }
                }
                Ok(false)
            }
            InputAction::PendingG | InputAction::Noop => Ok(false),
        }
    }

    fn switch_to_normal(&mut self) -> anyhow::Result<()> {
        self.mode = Mode::Normal;
        self.input_matcher = InputMatcher::new();
        render(self.mode, &self.statusline_config, self.rows - 1)?;
        let _ = std::io::stdout().flush();
        Ok(())
    }

    fn switch_to_insert(&mut self) -> anyhow::Result<()> {
        self.mode = Mode::Insert;
        self.input_matcher = InputMatcher::new();
        render(self.mode, &self.statusline_config, self.rows - 1)?;
        let _ = std::io::stdout().flush();
        Ok(())
    }
}
