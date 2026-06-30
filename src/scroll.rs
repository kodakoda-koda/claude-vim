use crate::input::ScrollAmount;

/// SGR マウスホイールイベントを PTY に送信してスクロールする構造体。
/// Claude Code は SGR マウスモード (\x1b[?1006h) を有効化しているため、
/// マウスホイールイベントを PTY に直接送ることで内部スクロールを制御できる。
pub struct Scroller {
    rows: u16,
    cols: u16,
}

// SGR マウスイベントのボタン番号
const MOUSE_SCROLL_UP: u8 = 64;
const MOUSE_SCROLL_DOWN: u8 = 65;

// 1 マウスイベントあたりの推定スクロール行数
const LINES_PER_EVENT: u16 = 3;

impl Scroller {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { rows, cols }
    }

    pub fn set_size(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
    }

    /// ScrollAmount に応じたバイト列を生成する。
    pub fn scroll_bytes(&self, amount: ScrollAmount) -> Vec<u8> {
        let col = self.cols / 2;
        let row = self.rows / 2;

        match amount {
            ScrollAmount::LineUp => sgr_scroll_event(MOUSE_SCROLL_UP, col, row),
            ScrollAmount::LineDown => sgr_scroll_event(MOUSE_SCROLL_DOWN, col, row),
            ScrollAmount::HalfPageUp => sgr_scroll_events(MOUSE_SCROLL_UP, col, row, half_page(self.rows)),
            ScrollAmount::HalfPageDown => sgr_scroll_events(MOUSE_SCROLL_DOWN, col, row, half_page(self.rows)),
            ScrollAmount::FullPageUp => sgr_scroll_events(MOUSE_SCROLL_UP, col, row, full_page(self.rows)),
            ScrollAmount::FullPageDown => sgr_scroll_events(MOUSE_SCROLL_DOWN, col, row, full_page(self.rows)),
            // gg → Ctrl+Home, G → Ctrl+End（マウスイベント大量送信の代わり）
            ScrollAmount::Top => b"\x1b[1;5H".to_vec(),
            ScrollAmount::Bottom => b"\x1b[1;5F".to_vec(),
        }
    }
}

fn half_page(rows: u16) -> u16 {
    (rows / 2 / LINES_PER_EVENT).max(1)
}

fn full_page(rows: u16) -> u16 {
    (rows / LINES_PER_EVENT).max(1)
}

/// 1回分の SGR マウスホイールイベント: \x1b[<button;col;rowM
fn sgr_scroll_event(button: u8, col: u16, row: u16) -> Vec<u8> {
    format!("\x1b[<{};{};{}M", button, col, row).into_bytes()
}

/// 複数回のマウスホイールイベントを連結
fn sgr_scroll_events(button: u8, col: u16, row: u16, count: u16) -> Vec<u8> {
    let single = sgr_scroll_event(button, col, row);
    let mut result = Vec::with_capacity(single.len() * count as usize);
    for _ in 0..count {
        result.extend_from_slice(&single);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sgr_scroll_event_format() {
        let event = sgr_scroll_event(64, 40, 12);
        assert_eq!(event, b"\x1b[<64;40;12M");
    }

    #[test]
    fn test_line_up() {
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::LineUp);
        assert_eq!(bytes, b"\x1b[<64;40;12M");
    }

    #[test]
    fn test_line_down() {
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::LineDown);
        assert_eq!(bytes, b"\x1b[<65;40;12M");
    }

    #[test]
    fn test_half_page_count() {
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::HalfPageUp);
        let single = b"\x1b[<64;40;12M";
        // 24/2/3 = 4 events
        assert_eq!(bytes.len(), single.len() * 4);
    }

    #[test]
    fn test_full_page_count() {
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::FullPageUp);
        let single = b"\x1b[<64;40;12M";
        // 24/3 = 8 events
        assert_eq!(bytes.len(), single.len() * 8);
    }

    #[test]
    fn test_half_page_scales_with_rows() {
        let s = Scroller::new(80, 48);
        let bytes = s.scroll_bytes(ScrollAmount::HalfPageDown);
        let single_len = b"\x1b[<65;40;24M".len();
        // 48/2/3 = 8 events
        assert_eq!(bytes.len(), single_len * 8);
    }

    #[test]
    fn test_small_window_min_1_event() {
        let s = Scroller::new(80, 4);
        let bytes = s.scroll_bytes(ScrollAmount::HalfPageUp);
        let single_len = b"\x1b[<64;40;2M".len();
        // 4/2/3 = 0 → max(1) = 1
        assert_eq!(bytes.len(), single_len * 1);
    }

    #[test]
    fn test_top_sends_ctrl_home() {
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::Top);
        assert_eq!(bytes, b"\x1b[1;5H");
    }

    #[test]
    fn test_bottom_sends_ctrl_end() {
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::Bottom);
        assert_eq!(bytes, b"\x1b[1;5F");
    }

    #[test]
    fn test_set_size() {
        let mut s = Scroller::new(80, 24);
        s.set_size(120, 40);
        let bytes = s.scroll_bytes(ScrollAmount::LineUp);
        assert_eq!(bytes, b"\x1b[<64;60;20M");
    }
}
