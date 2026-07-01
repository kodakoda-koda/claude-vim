/// スクロール量を表す enum（count は繰り返し回数、1 = デフォルト）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAmount {
    /// j
    LineDown(u32),
    /// k
    LineUp(u32),
    /// Ctrl+d（半ページ）
    HalfPageDown(u32),
    /// Ctrl+u
    HalfPageUp(u32),
    /// Ctrl+f
    FullPageDown(u32),
    /// Ctrl+b
    FullPageUp(u32),
    /// gg（count 無視）
    Top,
    /// G（count 無視）
    Bottom,
}

/// Normal mode でのキー入力に対するアクション
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    /// スクロールを実行する
    Scroll(ScrollAmount),
    /// Insert mode に切替える
    SwitchToInsert,
    /// Cursor mode に切替える
    EnterCursor,
    /// バイト列をそのまま PTY に透過する
    Passthrough(Vec<u8>),
    /// gg の1打目。PTY に送らず、次の入力を待つ
    PendingG,
    /// 何もしない（Normal mode で無視するキー）
    Noop,
}

/// Normal mode のキーマッチング状態（g g の二重打ち対応・数値プレフィックス対応）
pub struct InputMatcher {
    pending_g: bool,
    /// 数値プレフィックスの蓄積値。0 = 未入力
    count: u32,
}

impl InputMatcher {
    pub fn new() -> Self {
        Self { pending_g: false, count: 0 }
    }

    /// raw bytes を受け取り InputAction を返す純粋関数（副作用なし）。
    /// 戻り値は (action, new_state) のタプルで immutable に扱う。
    pub fn process(&self, bytes: &[u8]) -> (InputAction, Self) {
        if self.pending_g {
            match bytes {
                b"g" => (
                    InputAction::Scroll(ScrollAmount::Top),
                    Self { pending_g: false, count: 0 },
                ),
                _ => {
                    // pending_g をキャンセルして現在の入力を通常処理する
                    let new_self = Self { pending_g: false, count: self.count };
                    new_self.process(bytes)
                }
            }
        } else {
            // 数値プレフィックス: 先頭が '1'-'9' または count > 0 の時に '0'-'9'
            if let Some(&digit_byte) = bytes.first()
                && bytes.len() == 1
            {
                if (b'1'..=b'9').contains(&digit_byte) {
                    let digit = (digit_byte - b'0') as u32;
                    let new_count = if self.count == 0 {
                        digit
                    } else {
                        self.count.saturating_mul(10).saturating_add(digit)
                    };
                    return (InputAction::Noop, Self { pending_g: false, count: new_count });
                }
                if digit_byte == b'0' && self.count > 0 {
                    let new_count = self.count.saturating_mul(10);
                    return (InputAction::Noop, Self { pending_g: false, count: new_count });
                }
            }

            let n = if self.count == 0 { 1 } else { self.count };

            match bytes {
                b"j" => (InputAction::Scroll(ScrollAmount::LineDown(n)), Self::new()),
                b"k" => (InputAction::Scroll(ScrollAmount::LineUp(n)), Self::new()),
                b"\x04" | b"\x1b[100;5u" => (InputAction::Scroll(ScrollAmount::HalfPageDown(n)), Self::new()),
                b"\x15" | b"\x1b[117;5u" => (InputAction::Scroll(ScrollAmount::HalfPageUp(n)), Self::new()),
                b"\x06" | b"\x1b[102;5u" => (InputAction::Scroll(ScrollAmount::FullPageDown(n)), Self::new()),
                b"\x02" | b"\x1b[98;5u" => (InputAction::Scroll(ScrollAmount::FullPageUp(n)), Self::new()),
                b"G" => (InputAction::Scroll(ScrollAmount::Bottom), Self::new()),
                b"i" => (InputAction::SwitchToInsert, Self::new()),
                b"c" => (InputAction::EnterCursor, Self::new()),
                b"g" => (InputAction::PendingG, Self { pending_g: true, count: self.count }),
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

/// Cursor mode でのキー入力に対するアクション
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorInputAction {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    EnterVisual,
    Cancel,
    Noop,
}

/// Cursor mode のキーマッチング
pub struct CursorInputMatcher;

impl CursorInputMatcher {
    pub fn new() -> Self { Self }

    pub fn process(&self, bytes: &[u8]) -> CursorInputAction {
        match bytes {
            b"h" => CursorInputAction::MoveLeft,
            b"j" => CursorInputAction::MoveDown,
            b"k" => CursorInputAction::MoveUp,
            b"l" => CursorInputAction::MoveRight,
            b"v" => CursorInputAction::EnterVisual,
            b"\x1b" | b"\x1b[27u" => CursorInputAction::Cancel,
            _ => CursorInputAction::Noop,
        }
    }
}

impl Default for CursorInputMatcher {
    fn default() -> Self { Self::new() }
}

/// Visual mode でのキー入力に対するアクション
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualInputAction {
    MoveDown,
    MoveUp,
    MoveLeft,
    MoveRight,
    Yank,
    Cancel,
    Noop,
}

/// Visual mode のキーマッチング（ステートレス）
pub struct VisualInputMatcher;

impl VisualInputMatcher {
    pub fn new() -> Self {
        Self
    }

    pub fn process(&self, bytes: &[u8]) -> VisualInputAction {
        match bytes {
            b"j" => VisualInputAction::MoveDown,
            b"k" => VisualInputAction::MoveUp,
            b"h" => VisualInputAction::MoveLeft,
            b"l" => VisualInputAction::MoveRight,
            b"y" => VisualInputAction::Yank,
            b"\x1b" | b"\x1b[27u" => VisualInputAction::Cancel,
            _ => VisualInputAction::Noop,
        }
    }
}

impl Default for VisualInputMatcher {
    fn default() -> Self {
        Self::new()
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
        assert_eq!(action, InputAction::Scroll(ScrollAmount::LineDown(1)));
    }

    #[test]
    fn test_k_scrolls_line_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"k");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::LineUp(1)));
    }

    #[test]
    fn test_ctrl_d_scrolls_half_page_down() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x04");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::HalfPageDown(1)));
    }

    #[test]
    fn test_ctrl_u_scrolls_half_page_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x15");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::HalfPageUp(1)));
    }

    #[test]
    fn test_ctrl_f_scrolls_full_page_down() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x06");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::FullPageDown(1)));
    }

    #[test]
    fn test_kitty_ctrl_d_scrolls_half_page_down() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x1b[100;5u");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::HalfPageDown(1)));
    }

    #[test]
    fn test_kitty_ctrl_u_scrolls_half_page_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x1b[117;5u");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::HalfPageUp(1)));
    }

    #[test]
    fn test_kitty_ctrl_f_scrolls_full_page_down() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x1b[102;5u");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::FullPageDown(1)));
    }

    #[test]
    fn test_kitty_ctrl_b_scrolls_full_page_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x1b[98;5u");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::FullPageUp(1)));
    }

    #[test]
    fn test_ctrl_b_scrolls_full_page_up() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"\x02");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::FullPageUp(1)));
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
        let (action, _) = m.process(b"z");
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
    fn test_pending_g_then_j_scrolls() {
        // g → j: pending_g キャンセル、j を通常処理 → Scroll(LineDown(1))
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"g");
        let (action, m3) = m2.process(b"j");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::LineDown(1)));
        assert!(!m3.pending_g);
    }

    #[test]
    fn test_pending_g_then_i_switches_to_insert() {
        // g → i: pending_g キャンセル、i を通常処理 → SwitchToInsert
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"g");
        let (action, _) = m2.process(b"i");
        assert_eq!(action, InputAction::SwitchToInsert);
    }

    #[test]
    fn test_pending_g_then_printable_noop() {
        // g → z: pending_g キャンセル、z は printable → Noop
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"g");
        let (action, _) = m2.process(b"z");
        assert_eq!(action, InputAction::Noop);
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

    // 数値プレフィックスのテスト
    #[test]
    fn test_count_prefix_5j_scrolls_5_lines_down() {
        let m = InputMatcher::new();
        let (a1, m2) = m.process(b"5");
        assert_eq!(a1, InputAction::Noop);
        let (action, _) = m2.process(b"j");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::LineDown(5)));
    }

    #[test]
    fn test_count_prefix_10k_scrolls_10_lines_up() {
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"1");
        let (_, m3) = m2.process(b"0");
        let (action, _) = m3.process(b"k");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::LineUp(10)));
    }

    #[test]
    fn test_count_prefix_with_gg_ignored() {
        // 5 → g → g で Top（count 無視）
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"5");
        let (_, m3) = m2.process(b"g");
        let (action, _) = m3.process(b"g");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::Top));
    }

    #[test]
    fn test_count_prefix_with_g_ignored() {
        // 5 → G で Bottom（count 無視）
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"5");
        let (action, _) = m2.process(b"G");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::Bottom));
    }

    #[test]
    fn test_count_reset_after_command() {
        // 5 → j の後は count がリセット → 次の j は LineDown(1)
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"5");
        let (_, m3) = m2.process(b"j");
        let (action, _) = m3.process(b"j");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::LineDown(1)));
    }

    #[test]
    fn test_leading_zero_not_counted() {
        // '0' で count は開始しない → Noop（count は 0 のまま）
        let m = InputMatcher::new();
        let (action, m2) = m.process(b"0");
        assert_eq!(action, InputAction::Noop);
        assert_eq!(m2.count, 0);
    }

    #[test]
    fn test_count_prefix_with_ctrl_d() {
        // 3 → Ctrl+d で HalfPageDown(3)
        let m = InputMatcher::new();
        let (_, m2) = m.process(b"3");
        let (action, _) = m2.process(b"\x04");
        assert_eq!(action, InputAction::Scroll(ScrollAmount::HalfPageDown(3)));
    }

    // Visual mode のキーマッチングテスト
    #[test]
    fn test_c_enters_cursor_mode() {
        let m = InputMatcher::new();
        let (action, _) = m.process(b"c");
        assert_eq!(action, InputAction::EnterCursor);
    }

    #[test]
    fn test_cursor_hjkl() {
        let m = CursorInputMatcher::new();
        assert_eq!(m.process(b"h"), CursorInputAction::MoveLeft);
        assert_eq!(m.process(b"j"), CursorInputAction::MoveDown);
        assert_eq!(m.process(b"k"), CursorInputAction::MoveUp);
        assert_eq!(m.process(b"l"), CursorInputAction::MoveRight);
    }

    #[test]
    fn test_cursor_v_enters_visual() {
        let m = CursorInputMatcher::new();
        assert_eq!(m.process(b"v"), CursorInputAction::EnterVisual);
    }

    #[test]
    fn test_cursor_esc_cancels() {
        let m = CursorInputMatcher::new();
        assert_eq!(m.process(b"\x1b"), CursorInputAction::Cancel);
    }

    #[test]
    fn test_visual_hjkl() {
        let m = VisualInputMatcher::new();
        assert_eq!(m.process(b"h"), VisualInputAction::MoveLeft);
        assert_eq!(m.process(b"j"), VisualInputAction::MoveDown);
        assert_eq!(m.process(b"k"), VisualInputAction::MoveUp);
        assert_eq!(m.process(b"l"), VisualInputAction::MoveRight);
    }

    #[test]
    fn test_visual_y_yanks() {
        let m = VisualInputMatcher::new();
        assert_eq!(m.process(b"y"), VisualInputAction::Yank);
    }

    #[test]
    fn test_visual_esc_cancels() {
        let m = VisualInputMatcher::new();
        assert_eq!(m.process(b"\x1b"), VisualInputAction::Cancel);
    }

    #[test]
    fn test_visual_kitty_esc_cancels() {
        let m = VisualInputMatcher::new();
        assert_eq!(m.process(b"\x1b[27u"), VisualInputAction::Cancel);
    }

    #[test]
    fn test_visual_other_key_noop() {
        let m = VisualInputMatcher::new();
        assert_eq!(m.process(b"x"), VisualInputAction::Noop);
    }
}
