/// スクロール量を表す enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAmount {
    /// 1l — j
    LineDown,
    /// 1l- — k
    LineUp,
    /// {rows/2}l — Ctrl+d（半ページ。行数で指定）
    HalfPageDown,
    /// {rows/2}l- — Ctrl+u
    HalfPageUp,
    /// 1p — Ctrl+f
    FullPageDown,
    /// 1p- — Ctrl+b
    FullPageUp,
    /// start — gg
    Top,
    /// end — G
    Bottom,
}

/// Normal mode でのキー入力に対するアクション
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    /// スクロールを実行する
    Scroll(ScrollAmount),
    /// Insert mode に切替える
    SwitchToInsert,
    /// バイト列をそのまま PTY に透過する
    Passthrough(Vec<u8>),
    /// gg の1打目。PTY に送らず、次の入力を待つ
    PendingG,
    /// 何もしない（Normal mode で無視するキー）
    Noop,
}

/// Normal mode のキーマッチング状態（g g の二重打ち対応）
pub struct InputMatcher {
    pending_g: bool,
}

impl InputMatcher {
    pub fn new() -> Self {
        Self { pending_g: false }
    }

    /// raw bytes を受け取り InputAction を返す純粋関数（副作用なし）。
    /// 戻り値は (action, new_state) のタプルで immutable に扱う。
    pub fn process(&self, bytes: &[u8]) -> (InputAction, Self) {
        if self.pending_g {
            match bytes {
                b"g" => (
                    InputAction::Scroll(ScrollAmount::Top),
                    Self { pending_g: false },
                ),
                _ => (
                    // pending だった g を PTY に透過。App 側は現在の入力を再処理する
                    InputAction::Passthrough(b"g".to_vec()),
                    Self { pending_g: false },
                ),
            }
        } else {
            match bytes {
                b"j" => (InputAction::Scroll(ScrollAmount::LineDown), Self::new()),
                b"k" => (InputAction::Scroll(ScrollAmount::LineUp), Self::new()),
                b"\x04" | b"\x1b[100;5u" => (InputAction::Scroll(ScrollAmount::HalfPageDown), Self::new()),
                b"\x15" | b"\x1b[117;5u" => (InputAction::Scroll(ScrollAmount::HalfPageUp), Self::new()),
                b"\x06" | b"\x1b[102;5u" => (InputAction::Scroll(ScrollAmount::FullPageDown), Self::new()),
                b"\x02" | b"\x1b[98;5u" => (InputAction::Scroll(ScrollAmount::FullPageUp), Self::new()),
                b"G" => (InputAction::Scroll(ScrollAmount::Bottom), Self::new()),
                b"i" => (InputAction::SwitchToInsert, Self::new()),
                b"g" => (InputAction::PendingG, Self { pending_g: true }),
                _ => {
                    // Control chars (Enter, Ctrl+C 等) と escape sequences は PTY に透過
                    // Printable ASCII (0x20-0x7e) は Normal mode では無視
                    let first = bytes.first().copied().unwrap_or(0);
                    if first < 0x20 || first == 0x7f || first == 0x1b {
                        (InputAction::Passthrough(bytes.to_vec()), Self::new())
                    } else {
                        (InputAction::Noop, Self::new())
                    }
                }
            }
        }
    }
}

impl Default for InputMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_j_scrolls_line_down() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"j");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::LineDown));
    }

    #[test]
    fn test_k_scrolls_line_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"k");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::LineUp));
    }

    #[test]
    fn test_ctrl_d_scrolls_half_page_down() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x04");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::HalfPageDown));
    }

    #[test]
    fn test_ctrl_u_scrolls_half_page_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x15");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::HalfPageUp));
    }

    #[test]
    fn test_ctrl_f_scrolls_full_page_down() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x06");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::FullPageDown));
    }

    #[test]
    fn test_kitty_ctrl_d_scrolls_half_page_down() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x1b[100;5u");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::HalfPageDown));
    }

    #[test]
    fn test_kitty_ctrl_u_scrolls_half_page_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x1b[117;5u");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::HalfPageUp));
    }

    #[test]
    fn test_kitty_ctrl_f_scrolls_full_page_down() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x1b[102;5u");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::FullPageDown));
    }

    #[test]
    fn test_kitty_ctrl_b_scrolls_full_page_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x1b[98;5u");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::FullPageUp));
    }

    #[test]
    fn test_ctrl_b_scrolls_full_page_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x02");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::FullPageUp));
    }

    #[test]
    fn test_g_first_returns_pending_g() {
        let m = InputMatcher::new();
        let (action, new_m) = m.process(b"g");
        assert_eq!(action, InputAction::PendingG);
        assert!(new_m.pending_g);
    }

    #[test]
    fn test_gg_scrolls_to_top() {
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"g"); // first g → PendingG
        let (action, m3) = m2.process(b"g"); // second g → Top
        assert_eq!(action, InputAction::Scroll(ScrollAmount::Top));
        assert!(!m3.pending_g);
    }

    #[test]
    fn test_capital_g_scrolls_to_bottom() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"G");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::Bottom));
    }

    #[test]
    fn test_i_switches_to_insert() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"i");
        assert_eq!(action, InputAction::SwitchToInsert);
    }

    #[test]
    fn test_printable_key_noop() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"y");
        assert_eq!(action, InputAction::Noop);
    }

    #[test]
    fn test_ctrl_c_passthrough() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x03");
        assert_eq!(action, InputAction::Passthrough(b"\x03".to_vec()));
    }

    #[test]
    fn test_enter_passthrough() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\r");
        assert_eq!(action, InputAction::Passthrough(b"\r".to_vec()));
    }

    #[test]
    fn test_escape_sequence_passthrough() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x1b[A");
        assert_eq!(action, InputAction::Passthrough(b"\x1b[A".to_vec()));
    }

    #[test]
    fn test_pending_g_then_other_key_passthrough_g() {
        // g → j: pending g をパススルーし、新しい状態は pending_g: false
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"g"); // pending_g = true
        let (action, m3) = m2.process(b"j"); // not g → Passthrough("g")
        assert_eq!(action, InputAction::Passthrough(b"g".to_vec()));
        assert!(!m3.pending_g);
    }

    #[test]
    fn test_immutable_original_unchanged() {
        // process は &self を受け取り元の状態を変えない
        let m = InputMatcher::new();
        let (_, _m2) = m.process(b"g");
        // m はまだ pending_g: false のまま
        assert!(!m.pending_g);
    }

    #[test]
    fn test_new_state_from_process_is_independent() {
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"j");
        // j は状態を変えない
        assert!(!m2.pending_g);
    }
}
