use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use crate::screen::VirtualScreen;

/// smcup/rmcup などの alternate screen シーケンスをフィルタして除去する純粋関数。
/// 入力バイト列から特定のエスケープシーケンスを削除した新しい Vec<u8> を返す。
/// フィルタ対象:
///   \x1b[?1049h, \x1b[?1049l  (smcup/rmcup)
///   \x1b[?1047h, \x1b[?1047l  (旧形式)
///   \x1b[?47h,   \x1b[?47l    (さらに旧形式)
///   \x1b[3J                    (スクロールバック消去)
pub fn filter_smcup(bytes: &[u8]) -> Vec<u8> {
    // フィルタ対象シーケンス（長い方を先に並べる）
    const FILTER_SEQS: &[&[u8]] = &[
        b"\x1b[?1049h",
        b"\x1b[?1049l",
        b"\x1b[?1047h",
        b"\x1b[?1047l",
        b"\x1b[?47h",
        b"\x1b[?47l",
        b"\x1b[3J",
    ];

    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;

    'outer: while i < bytes.len() {
        for seq in FILTER_SEQS {
            if bytes[i..].starts_with(seq) {
                i += seq.len();
                continue 'outer;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }

    result
}

/// PTY セッションを保持する構造体
pub struct PtySession {
    master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>,
    output_rx: Receiver<Vec<u8>>,
    /// VirtualScreen への共有参照（Visual mode で行テキストを取得するため）
    pub screen: Arc<Mutex<VirtualScreen>>,
}

impl PtySession {
    /// claude サブプロセスを PTY で起動する。
    /// cols/rows はステータスライン分を引いた値（実際の端末行数 - 1）。
    pub fn spawn(cols: u16, rows: u16) -> anyhow::Result<Self> {
        use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
        use std::sync::mpsc;

        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let cmd = CommandBuilder::new("claude");
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let writer = pair.master.take_writer()?;
        let writer = Arc::new(Mutex::new(writer));

        let (tx, rx) = mpsc::channel::<Vec<u8>>();

        // VirtualScreen を生成
        let screen = Arc::new(Mutex::new(VirtualScreen::new(cols as usize, rows as usize)));
        let screen_clone = Arc::clone(&screen);

        // PTY reader スレッド
        let mut reader = pair.master.try_clone_reader()?;
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let filtered = filter_smcup(&buf[..n]);
                        {
                            if let Ok(mut s) = screen_clone.lock() {
                                s.feed(&filtered);
                            }
                        }
                        if tx.send(filtered).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let master = Arc::new(Mutex::new(pair.master));

        Ok(Self {
            master,
            child,
            writer,
            output_rx: rx,
            screen,
        })
    }

    /// PTY から読み取ったバイト列を受信するチャンネルを返す
    pub fn output_rx(&self) -> &Receiver<Vec<u8>> {
        &self.output_rx
    }

    /// PTY にバイト列を書き込む
    pub fn write_bytes(&self, bytes: &[u8]) -> anyhow::Result<()> {
        use std::io::Write;
        let mut w = self.writer.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        w.write_all(bytes)?;
        Ok(())
    }

    /// PTY のサイズを変更する
    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        use portable_pty::PtySize;
        let master = self.master.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        if let Ok(mut s) = self.screen.lock() {
            s.resize(cols as usize, rows as usize);
        }
        Ok(())
    }

    /// claude プロセスが終了したか確認する
    pub fn is_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_smcup_removes_1049h() {
        let input = b"\x1b[?1049hSOME TEXT";
        let result = filter_smcup(input);
        assert_eq!(result, b"SOME TEXT");
    }

    #[test]
    fn test_filter_smcup_removes_1049l() {
        let input = b"\x1b[?1049lSOME TEXT";
        let result = filter_smcup(input);
        assert_eq!(result, b"SOME TEXT");
    }

    #[test]
    fn test_filter_smcup_removes_1047h() {
        let input = b"\x1b[?1047h";
        let result = filter_smcup(input);
        assert_eq!(result, b"");
    }

    #[test]
    fn test_filter_smcup_removes_1047l() {
        let input = b"\x1b[?1047l";
        let result = filter_smcup(input);
        assert_eq!(result, b"");
    }

    #[test]
    fn test_filter_smcup_removes_47h() {
        let input = b"\x1b[?47h";
        let result = filter_smcup(input);
        assert_eq!(result, b"");
    }

    #[test]
    fn test_filter_smcup_removes_47l() {
        let input = b"\x1b[?47l";
        let result = filter_smcup(input);
        assert_eq!(result, b"");
    }

    #[test]
    fn test_filter_smcup_removes_3j() {
        let input = b"\x1b[3J";
        let result = filter_smcup(input);
        assert_eq!(result, b"");
    }

    #[test]
    fn test_filter_smcup_passes_through_2j() {
        let input = b"\x1b[2J";
        let result = filter_smcup(input);
        assert_eq!(result, b"\x1b[2J");
    }

    #[test]
    fn test_filter_smcup_passes_through_normal_text() {
        let input = b"hello world";
        let result = filter_smcup(input);
        assert_eq!(result, b"hello world");
    }

    #[test]
    fn test_filter_smcup_empty_input() {
        let result = filter_smcup(b"");
        assert_eq!(result, b"");
    }

    #[test]
    fn test_filter_smcup_multiple_sequences() {
        let input = b"\x1b[?1049hHELLO\x1b[3JWORLD\x1b[?1049l";
        let result = filter_smcup(input);
        assert_eq!(result, b"HELLOWORLD");
    }

    #[test]
    fn test_filter_smcup_preserves_other_escape_sequences() {
        // \x1b[2J (clear screen) should pass through
        let input = b"\x1b[2J\x1b[H";
        let result = filter_smcup(input);
        assert_eq!(result, b"\x1b[2J\x1b[H");
    }

    #[test]
    fn test_filter_smcup_1047h_not_confused_with_47h() {
        // \x1b[?1047h should be removed (longer sequence matched first)
        let input = b"\x1b[?1047hTEXT";
        let result = filter_smcup(input);
        assert_eq!(result, b"TEXT");
    }
}
