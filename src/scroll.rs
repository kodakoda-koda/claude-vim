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
    /// count が含まれている場合はその分のイベントを連結して返す。
    pub fn scroll_bytes(&self, amount: ScrollAmount) -> Vec<u8> {
        let col = (self.cols / 2).max(1);
        let row = (self.rows / 2).max(1);
        let clamp = |n: u32| -> u16 { n.min(u16::MAX as u32) as u16 };
        let clamp_mul = |a: u16, b: u32| -> u16 { (a as u32).saturating_mul(b).min(u16::MAX as u32) as u16 };

        match amount {
            ScrollAmount::LineUp(count) => {
                sgr_scroll_events(MOUSE_SCROLL_UP, col, row, clamp(count))
            }
            ScrollAmount::LineDown(count) => {
                sgr_scroll_events(MOUSE_SCROLL_DOWN, col, row, clamp(count))
            }
            ScrollAmount::HalfPageUp(count) => {
                sgr_scroll_events(MOUSE_SCROLL_UP, col, row, clamp_mul(half_page(self.rows), count))
            }
            ScrollAmount::HalfPageDown(count) => {
                sgr_scroll_events(MOUSE_SCROLL_DOWN, col, row, clamp_mul(half_page(self.rows), count))
            }
            ScrollAmount::FullPageUp(count) => {
                sgr_scroll_events(MOUSE_SCROLL_UP, col, row, clamp_mul(full_page(self.rows), count))
            }
            ScrollAmount::FullPageDown(count) => {
                sgr_scroll_events(MOUSE_SCROLL_DOWN, col, row, clamp_mul(full_page(self.rows), count))
            }
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
        let bytes = s.scroll_bytes(ScrollAmount::LineUp(1));
        assert_eq!(bytes, b"\x1b[<64;40;12M");
    }

    #[test]
    fn test_line_down() {
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::LineDown(1));
        assert_eq!(bytes, b"\x1b[<65;40;12M");
    }

    #[test]
    fn test_line_down_count_3() {
        // LineDown(3) は 3 回分のイベントを生成する
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::LineDown(3));
        let single = b"\x1b[<65;40;12M";
        assert_eq!(bytes.len(), single.len() * 3);
    }

    #[test]
    fn test_half_page_count() {
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::HalfPageUp(1));
        let single = b"\x1b[<64;40;12M";
        // 24/2/3 = 4 events
        assert_eq!(bytes.len(), single.len() * 4);
    }

    #[test]
    fn test_half_page_count_3() {
        // HalfPageDown(3) は 3 倍の半ページイベントを生成する
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::HalfPageDown(3));
        let single = b"\x1b[<65;40;12M";
        // (24/2/3) * 3 = 12 events
        assert_eq!(bytes.len(), single.len() * 12);
    }

    #[test]
    fn test_full_page_count() {
        let s = Scroller::new(80, 24);
        let bytes = s.scroll_bytes(ScrollAmount::FullPageUp(1));
        let single = b"\x1b[<64;40;12M";
        // 24/3 = 8 events
        assert_eq!(bytes.len(), single.len() * 8);
    }

    #[test]
    fn test_half_page_scales_with_rows() {
        let s = Scroller::new(80, 48);
        let bytes = s.scroll_bytes(ScrollAmount::HalfPageDown(1));
        let single_len = b"\x1b[<65;40;24M".len();
        // 48/2/3 = 8 events
        assert_eq!(bytes.len(), single_len * 8);
    }

    #[test]
    fn test_small_window_min_1_event() {
        let s = Scroller::new(80, 4);
        let bytes = s.scroll_bytes(ScrollAmount::HalfPageUp(1));
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
        let bytes = s.scroll_bytes(ScrollAmount::LineUp(1));
        assert_eq!(bytes, b"\x1b[<64;60;20M");
    }
}
