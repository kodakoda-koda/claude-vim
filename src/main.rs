mod app;
mod input;
mod scroll;
mod pty;
mod screen;
mod statusline;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use app::App;
use scroll::Scroller;
use pty::PtySession;
use statusline::{Mode, StatuslineConfig, get_git_branch, render};

fn main() -> anyhow::Result<()> {
    // 1. ターミナルサイズ取得
    let (cols, rows) = crossterm::terminal::size()?;

    // 3. raw mode 有効化
    crossterm::terminal::enable_raw_mode()?;

    let result = run_app(cols, rows);

    // 9. 画面クリア + raw mode 解除
    {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = write!(stdout, "\x1b[2J\x1b[H");
        let _ = stdout.flush();
    }
    let _ = crossterm::terminal::disable_raw_mode();

    result
}

fn run_app(cols: u16, rows: u16) -> anyhow::Result<()> {
    // 4. git ブランチ取得
    let branch = get_git_branch();

    // 5. PTY 起動（ステータスライン分 -1 行）
    let pty = PtySession::spawn(cols, rows.saturating_sub(1))?;

    // 6. SIGWINCH フラグ登録
    let sigwinch_flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(
        signal_hook::consts::SIGWINCH,
        Arc::clone(&sigwinch_flag),
    )?;

    // 7. 画面クリア
    {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        write!(stdout, "\x1b[2J\x1b[H")?;
        stdout.flush()?;
    }

    // ステータスライン設定と初期描画
    let statusline_config = StatuslineConfig::new(cols, branch, false);
    render(Mode::Insert, &statusline_config, rows.saturating_sub(1))?;

    // 8. Scroller 構築（SGR マウスホイールイベントで PTY にスクロールを送る）
    let scroller = Scroller::new(cols, rows.saturating_sub(1));

    // App 起動
    let app = App::new(pty, scroller, statusline_config, sigwinch_flag, cols, rows)?;
    app.run()
}
