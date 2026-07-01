use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crate::input::{InputAction, InputMatcher, CursorInputAction, CursorInputMatcher, VisualInputAction, VisualInputMatcher};
use crate::scroll::Scroller;
use crate::pty::PtySession;
use crate::statusline::{Mode, StatuslineConfig, render};

const ESC_TIMEOUT_MS: u64 = 50;
const POLL_INTERVAL_MS: u64 = 5;
const SCROLL_THROTTLE_MS: u64 = 50;

/// Cursor/Visual mode での状態
#[derive(Debug, Clone)]
struct CursorState {
    cursor_row: usize,
    cursor_col: usize,
    /// Visual mode の選択開始位置（None = Cursor mode）
    anchor_row: Option<usize>,
    anchor_col: usize,
    /// 現在ハイライトされている行範囲（復元用）
    highlighted_lo: usize,
    highlighted_hi: usize,
}

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
    /// Cursor/Visual mode の状態
    cursor_state: Option<CursorState>,
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
            cursor_state: None,
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
                let _ = render(self.mode, &self.statusline_config, rows.saturating_sub(1));
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
                let _ = render(self.mode, &self.statusline_config, self.rows.saturating_sub(1));
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
            Mode::Cursor => self.handle_cursor_input(bytes),
            Mode::Visual => self.handle_visual_input(bytes),
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
            InputAction::EnterCursor => {
                self.switch_to_cursor()?;
                Ok(false)
            }
            InputAction::Passthrough(data) => {
                self.pty.write_bytes(&data)?;
                Ok(false)
            }
            InputAction::PendingG | InputAction::Noop => Ok(false),
        }
    }

    /// Cursor mode の入力を処理する
    fn handle_cursor_input(&mut self, bytes: Vec<u8>) -> anyhow::Result<bool> {
        let action = CursorInputMatcher::new().process(&bytes);
        let grid_rows = self.rows.saturating_sub(1) as usize;
        let grid_cols = self.cols as usize;
        match action {
            CursorInputAction::MoveDown => {
                if let Some(ref mut state) = self.cursor_state {
                    state.cursor_row = (state.cursor_row + 1).min(grid_rows.saturating_sub(1));
                }
                self.redraw_cursor_highlight()?;
                Ok(false)
            }
            CursorInputAction::MoveUp => {
                if let Some(ref mut state) = self.cursor_state {
                    state.cursor_row = state.cursor_row.saturating_sub(1);
                }
                self.redraw_cursor_highlight()?;
                Ok(false)
            }
            CursorInputAction::MoveRight => {
                if let Some(ref mut state) = self.cursor_state {
                    state.cursor_col = (state.cursor_col + 1).min(grid_cols.saturating_sub(1));
                }
                self.redraw_cursor_highlight()?;
                Ok(false)
            }
            CursorInputAction::MoveLeft => {
                if let Some(ref mut state) = self.cursor_state {
                    state.cursor_col = state.cursor_col.saturating_sub(1);
                }
                self.redraw_cursor_highlight()?;
                Ok(false)
            }
            CursorInputAction::EnterVisual => {
                if let Some(ref mut state) = self.cursor_state {
                    state.anchor_row = Some(state.cursor_row);
                    state.anchor_col = state.cursor_col;
                }
                self.mode = Mode::Visual;
                render(self.mode, &self.statusline_config, self.rows.saturating_sub(1))?;
                self.redraw_cursor_highlight()?;
                let _ = std::io::stdout().flush();
                Ok(false)
            }
            CursorInputAction::Cancel => {
                self.restore_highlighted_rows()?;
                self.switch_to_normal()?;
                Ok(false)
            }
            CursorInputAction::Noop => Ok(false),
        }
    }

    /// Visual mode の入力を処理する
    fn handle_visual_input(&mut self, bytes: Vec<u8>) -> anyhow::Result<bool> {
        let action = VisualInputMatcher::new().process(&bytes);
        let grid_rows = self.rows.saturating_sub(1) as usize;
        let grid_cols = self.cols as usize;
        match action {
            VisualInputAction::MoveDown => {
                if let Some(ref mut state) = self.cursor_state {
                    state.cursor_row = (state.cursor_row + 1).min(grid_rows.saturating_sub(1));
                }
                self.redraw_cursor_highlight()?;
                Ok(false)
            }
            VisualInputAction::MoveUp => {
                if let Some(ref mut state) = self.cursor_state {
                    state.cursor_row = state.cursor_row.saturating_sub(1);
                }
                self.redraw_cursor_highlight()?;
                Ok(false)
            }
            VisualInputAction::MoveRight => {
                if let Some(ref mut state) = self.cursor_state {
                    state.cursor_col = (state.cursor_col + 1).min(grid_cols.saturating_sub(1));
                }
                self.redraw_cursor_highlight()?;
                Ok(false)
            }
            VisualInputAction::MoveLeft => {
                if let Some(ref mut state) = self.cursor_state {
                    state.cursor_col = state.cursor_col.saturating_sub(1);
                }
                self.redraw_cursor_highlight()?;
                Ok(false)
            }
            VisualInputAction::Yank => {
                self.do_yank()?;
                self.restore_highlighted_rows()?;
                self.switch_to_normal()?;
                Ok(false)
            }
            VisualInputAction::Cancel => {
                self.restore_highlighted_rows()?;
                self.switch_to_normal()?;
                Ok(false)
            }
            VisualInputAction::Noop => Ok(false),
        }
    }

    fn switch_to_normal(&mut self) -> anyhow::Result<()> {
        self.mode = Mode::Normal;
        self.input_matcher = InputMatcher::new();
        self.cursor_state = None;
        render(self.mode, &self.statusline_config, self.rows.saturating_sub(1))?;
        let _ = std::io::stdout().flush();
        Ok(())
    }

    fn switch_to_insert(&mut self) -> anyhow::Result<()> {
        self.mode = Mode::Insert;
        self.input_matcher = InputMatcher::new();
        render(self.mode, &self.statusline_config, self.rows.saturating_sub(1))?;
        let _ = std::io::stdout().flush();
        Ok(())
    }

    fn switch_to_cursor(&mut self) -> anyhow::Result<()> {
        let grid_rows = self.rows.saturating_sub(1) as usize;
        let start_row = grid_rows / 2;
        self.cursor_state = Some(CursorState {
            cursor_row: start_row,
            cursor_col: 0,
            anchor_row: None,
            anchor_col: 0,
            highlighted_lo: start_row,
            highlighted_hi: start_row,
        });
        self.mode = Mode::Cursor;
        render(self.mode, &self.statusline_config, self.rows.saturating_sub(1))?;
        self.redraw_cursor_highlight()?;
        let _ = std::io::stdout().flush();
        Ok(())
    }

    /// 前のハイライトを復元してから新しいハイライトを描画する
    fn redraw_cursor_highlight(&mut self) -> anyhow::Result<()> {
        use crossterm::{cursor::{MoveTo, RestorePosition, SavePosition}, execute, style::Print};

        let state = match &self.cursor_state {
            Some(s) => s.clone(),
            None => return Ok(()),
        };
        let grid_rows = self.rows.saturating_sub(1) as usize;
        let cols = self.cols as usize;

        // VirtualScreen から必要な行テキストをコピーしてすぐ unlock
        let lines: Vec<(usize, String)> = {
            let screen = self.pty.screen.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            let lo = state.highlighted_lo;
            let hi = state.highlighted_hi.min(grid_rows.saturating_sub(1));
            let new_lo = match state.anchor_row {
                None => state.cursor_row,
                Some(a) => a.min(state.cursor_row),
            };
            let new_hi = match state.anchor_row {
                None => state.cursor_row,
                Some(a) => a.max(state.cursor_row),
            }.min(grid_rows.saturating_sub(1));
            let all_lo = lo.min(new_lo);
            let all_hi = hi.max(new_hi);
            (all_lo..=all_hi).map(|r| (r, screen.screen_line(r))).collect()
        };

        let mut stdout = std::io::stdout();
        execute!(stdout, SavePosition)?;

        // 1. 前のハイライト行を復元
        for &(row, ref text) in &lines {
            if row >= state.highlighted_lo && row <= state.highlighted_hi.min(grid_rows.saturating_sub(1)) {
                let padded = format!("{:<width$}", text, width = cols);
                execute!(stdout, MoveTo(0, row as u16), Print(&padded))?;
            }
        }

        // 2. 新しいハイライトを描画
        match state.anchor_row {
            None => {
                let row = state.cursor_row.min(grid_rows.saturating_sub(1));
                let text = lines.iter().find(|(r, _)| *r == row).map(|(_, t)| t.as_str()).unwrap_or("");
                let chars: Vec<char> = text.chars().collect();
                let col = state.cursor_col.min(cols.saturating_sub(1));
                let before: String = chars[..col.min(chars.len())].iter().collect();
                let cursor_ch = chars.get(col).copied().unwrap_or(' ');
                let after: String = if col + 1 < chars.len() { chars[col + 1..].iter().collect() } else { String::new() };
                let remaining = cols.saturating_sub(before.chars().count() + 1 + after.chars().count());

                execute!(
                    stdout,
                    MoveTo(0, row as u16),
                    Print(format!("{before}\x1b[7m{cursor_ch}\x1b[27m{after}{}", " ".repeat(remaining))),
                )?;

                if let Some(ref mut s) = self.cursor_state {
                    s.highlighted_lo = row;
                    s.highlighted_hi = row;
                }
            }
            Some(anchor_row) => {
                let (start_row, start_col, end_row, end_col) =
                    if (anchor_row, state.anchor_col) <= (state.cursor_row, state.cursor_col) {
                        (anchor_row, state.anchor_col, state.cursor_row, state.cursor_col)
                    } else {
                        (state.cursor_row, state.cursor_col, anchor_row, state.anchor_col)
                    };

                for row in start_row..=end_row.min(grid_rows.saturating_sub(1)) {
                    let text = lines.iter().find(|(r, _)| *r == row).map(|(_, t)| t.as_str()).unwrap_or("");
                    let padded = format!("{:<width$}", text, width = cols);
                    let chars: Vec<char> = padded.chars().collect();
                    let hl_start = if row == start_row { start_col } else { 0 };
                    let hl_end = if row == end_row { (end_col + 1).min(cols) } else { cols };
                    let before: String = chars[..hl_start.min(chars.len())].iter().collect();
                    let highlighted: String = chars[hl_start.min(chars.len())..hl_end.min(chars.len())].iter().collect();
                    let after: String = chars[hl_end.min(chars.len())..].iter().collect();

                    execute!(
                        stdout,
                        MoveTo(0, row as u16),
                        Print(format!("{before}\x1b[7m{highlighted}\x1b[27m{after}")),
                    )?;
                }

                if let Some(ref mut s) = self.cursor_state {
                    s.highlighted_lo = start_row;
                    s.highlighted_hi = end_row;
                }
            }
        }

        execute!(stdout, RestorePosition)?;
        stdout.flush()?;
        Ok(())
    }

    /// ハイライトされた行を VirtualScreen のオリジナル内容で復元する
    fn restore_highlighted_rows(&self) -> anyhow::Result<()> {
        use crossterm::{cursor::{MoveTo, RestorePosition, SavePosition}, execute, style::Print};

        let state = match &self.cursor_state {
            Some(s) => s,
            None => return Ok(()),
        };
        let grid_rows = self.rows.saturating_sub(1) as usize;
        let cols = self.cols as usize;

        let lines: Vec<(usize, String)> = {
            let screen = self.pty.screen.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            (state.highlighted_lo..=state.highlighted_hi.min(grid_rows.saturating_sub(1)))
                .map(|r| (r, screen.screen_line(r)))
                .collect()
        };

        let mut stdout = std::io::stdout();
        execute!(stdout, SavePosition)?;

        for (row, text) in &lines {
            let padded = format!("{:<width$}", text, width = cols);
            execute!(stdout, MoveTo(0, *row as u16), Print(padded))?;
        }

        execute!(stdout, RestorePosition)?;
        stdout.flush()?;
        Ok(())
    }

    /// yank: 選択テキストをクリップボードにコピーする（文字単位）
    fn do_yank(&mut self) -> anyhow::Result<()> {
        let state = match &self.cursor_state {
            Some(s) => s.clone(),
            None => return Ok(()),
        };
        let anchor_row = match state.anchor_row {
            Some(a) => a,
            None => return Ok(()),
        };
        let cols = self.cols as usize;
        let grid_rows = self.rows.saturating_sub(1) as usize;
        let screen = self.pty.screen.lock().map_err(|e| anyhow::anyhow!("{}", e))?;

        let (start_row, start_col, end_row, end_col) =
            if (anchor_row, state.anchor_col) <= (state.cursor_row, state.cursor_col) {
                (anchor_row, state.anchor_col, state.cursor_row, state.cursor_col)
            } else {
                (state.cursor_row, state.cursor_col, anchor_row, state.anchor_col)
            };

        let mut result = String::new();
        for row in start_row..=end_row.min(grid_rows.saturating_sub(1)) {
            let text = screen.screen_line(row);
            let chars: Vec<char> = text.chars().collect();
            let from = if row == start_row { start_col } else { 0 };
            let to = if row == end_row { (end_col + 1).min(chars.len()) } else { cols.min(chars.len()) };
            let slice: String = chars[from.min(chars.len())..to.min(chars.len())].iter().collect();
            if row > start_row {
                result.push('\n');
            }
            result.push_str(slice.trim_end());
        }

        drop(screen);
        yank(&result)?;
        Ok(())
    }
}

/// テキストをクリップボードにコピーする
fn yank(text: &str) -> anyhow::Result<()> {
    yank_impl(text)
}

#[cfg(target_os = "macos")]
fn yank_impl(text: &str) -> anyhow::Result<()> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("pbcopy failed with exit code: {:?}", status.code());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn yank_impl(text: &str) -> anyhow::Result<()> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let xclip = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match xclip {
        Ok(mut child) => {
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(text.as_bytes())?;
            }
            let status = child.wait()?;
            if !status.success() {
                anyhow::bail!("xclip failed with exit code: {:?}", status.code());
            }
            Ok(())
        }
        Err(_) => {
            let mut child = Command::new("xsel")
                .args(["--clipboard", "--input"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| anyhow::anyhow!("yank: xclip and xsel not found: {}", e))?;
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(text.as_bytes())?;
            }
            let status = child.wait()?;
            if !status.success() {
                anyhow::bail!("xsel failed with exit code: {:?}", status.code());
            }
            Ok(())
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn yank_impl(_text: &str) -> anyhow::Result<()> {
    anyhow::bail!("yank: unsupported platform")
}
