use crossterm::{
    cursor::{MoveTo, RestorePosition, SavePosition},
    execute,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
};
use std::io::stdout;

const VERSION: &str = "cv v0.2";

/// cv の動作モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Insert,
    Normal,
    Cursor,
    Visual,
}

/// ステータスライン描画の設定
pub struct StatuslineConfig {
    pub cols: u16,
    pub branch: String,
    pub version: &'static str,
    pub use_powerline: bool,
}

impl StatuslineConfig {
    pub fn new(cols: u16, branch: String, use_powerline: bool) -> Self {
        Self {
            cols,
            branch,
            version: VERSION,
            use_powerline,
        }
    }
}

/// 現在の git ブランチ名を取得する。取得失敗時は空文字列を返す
pub fn get_git_branch() -> String {
    let output = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

/// ステータスラインの文字列表現を生成する純粋関数（描画なし）。
/// テスト用に公開する。
pub fn build_statusline(mode: Mode, config: &StatuslineConfig) -> String {
    let sep = if config.use_powerline {
        "\u{e0b0}"
    } else {
        "|"
    };

    let mode_str = match mode {
        Mode::Normal => " NORMAL ",
        Mode::Insert => " INSERT ",
        Mode::Cursor => " CURSOR ",
        Mode::Visual => " VISUAL ",
    };

    let branch_part = if config.branch.is_empty() {
        String::new()
    } else {
        format!(" {} {} ", sep, config.branch)
    };

    let left = format!("{}{}", mode_str, branch_part);
    let right = format!(" {} ", config.version);

    let total_cols = config.cols as usize;
    let left_len = left.chars().count();
    let right_len = right.chars().count();
    let padding = if total_cols > left_len + right_len {
        total_cols - left_len - right_len
    } else {
        0
    };

    let result = format!("{}{}{}", left, " ".repeat(padding), right);
    // cols を超えないようにトランケート
    result.chars().take(total_cols).collect()
}

/// ステータスラインをターミナル最下行に描画する。
/// crossterm で stdout に直接書き込む。
/// 行番号は引数で受け取る（cols は config から）。
pub fn render(mode: Mode, config: &StatuslineConfig, row: u16) -> anyhow::Result<()> {
    let content = build_statusline(mode, config);

    let (bg_color, fg_color) = match mode {
        Mode::Normal => (Color::Rgb { r: 100, g: 149, b: 237 }, Color::Rgb { r: 0, g: 0, b: 0 }),
        Mode::Insert => (Color::Rgb { r: 80, g: 200, b: 120 }, Color::Rgb { r: 0, g: 0, b: 0 }),
        Mode::Cursor => (Color::Rgb { r: 220, g: 180, b: 50 }, Color::Rgb { r: 0, g: 0, b: 0 }),
        Mode::Visual => (Color::Rgb { r: 190, g: 80, b: 190 }, Color::Rgb { r: 0, g: 0, b: 0 }),
    };

    execute!(
        stdout(),
        SavePosition,
        MoveTo(0, row),
        SetBackgroundColor(bg_color),
        SetForegroundColor(fg_color),
        Print(&content),
        ResetColor,
        RestorePosition,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(cols: u16, branch: &str, use_powerline: bool) -> StatuslineConfig {
        StatuslineConfig {
            cols,
            branch: branch.to_string(),
            version: VERSION,
            use_powerline,
        }
    }

    #[test]
    fn test_normal_mode_contains_normal() {
        let config = make_config(80, "main", false);
        let s = build_statusline(Mode::Normal, &config);
        assert!(s.contains("NORMAL"), "Expected 'NORMAL' in: {}", s);
    }

    #[test]
    fn test_insert_mode_contains_insert() {
        let config = make_config(80, "main", false);
        let s = build_statusline(Mode::Insert, &config);
        assert!(s.contains("INSERT"), "Expected 'INSERT' in: {}", s);
    }

    #[test]
    fn test_branch_name_included() {
        let config = make_config(80, "feat/v0.1", false);
        let s = build_statusline(Mode::Normal, &config);
        assert!(s.contains("feat/v0.1"), "Expected branch in: {}", s);
    }

    #[test]
    fn test_version_included() {
        let config = make_config(80, "main", false);
        let s = build_statusline(Mode::Normal, &config);
        assert!(s.contains("cv v0.2"), "Expected version in: {}", s);
    }

    #[test]
    fn test_no_powerline_uses_ascii_separator() {
        let config = make_config(80, "main", false);
        let s = build_statusline(Mode::Normal, &config);
        assert!(s.contains('|'), "Expected '|' separator in: {}", s);
    }

    #[test]
    fn test_powerline_uses_nerd_font_separator() {
        let config = make_config(80, "main", true);
        let s = build_statusline(Mode::Normal, &config);
        assert!(
            s.contains('\u{e0b0}'),
            "Expected Nerd Font separator in: {}",
            s
        );
    }

    #[test]
    fn test_empty_branch_no_separator() {
        let config = make_config(80, "", false);
        let s = build_statusline(Mode::Normal, &config);
        // ブランチが空のときはセパレーターが出ない
        assert!(!s.contains('|'), "Expected no separator when branch empty: {}", s);
    }

    #[test]
    fn test_statusline_length_matches_cols() {
        let config = make_config(80, "feat/v0.1", false);
        let s = build_statusline(Mode::Normal, &config);
        // 文字数が cols 以上であること（短すぎない）
        assert!(
            s.chars().count() <= 80,
            "Statusline too long: {} chars",
            s.chars().count()
        );
    }

    #[test]
    fn test_visual_mode_contains_visual() {
        let config = make_config(80, "main", false);
        let s = build_statusline(Mode::Visual, &config);
        assert!(s.contains("VISUAL"), "Expected 'VISUAL' in: {}", s);
    }
}
