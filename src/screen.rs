use std::collections::VecDeque;
use std::mem;

const MAX_SCROLLBACK: usize = 10_000;

/// 1セルの内容（文字とスタイル情報）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub bold: bool,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

impl Cell {
    pub fn blank() -> Self {
        Self {
            ch: ' ',
            bold: false,
            fg: None,
            bg: None,
        }
    }
}

/// ANSI 16色 + RGB
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Ansi(u8),
    Rgb(u8, u8, u8),
}

/// ターミナルの仮想画面。PTY 出力を vte でパースして状態を保持する。
pub struct VirtualScreen {
    /// 通常スクリーンのグリッド（行 × 列）
    grid: Vec<Vec<Cell>>,
    /// alternate screen グリッド（smcup 中に使用）
    alt_grid: Vec<Vec<Cell>>,
    /// スクロールバック（スクロールアウトした行のテキスト）
    pub scrollback: VecDeque<String>,
    /// カーソル位置（0-indexed, (col, row)）
    cursor: (usize, usize),
    /// 保存済みカーソル位置
    saved_cursor: (usize, usize),
    /// スクロール領域（top, bottom, 0-indexed inclusive）
    scroll_region: (usize, usize),
    /// alternate screen モードか
    in_alt_screen: bool,
    /// ターミナルサイズ
    rows: usize,
    cols: usize,
    /// 現在の属性（次の文字に適用）
    current_bold: bool,
    current_fg: Option<Color>,
    current_bg: Option<Color>,
    /// vte パーサー（チャンク断片またぎのためフィールドとして保持）
    parser: vte::Parser,
}

impl VirtualScreen {
    /// 新しい VirtualScreen を生成する
    pub fn new(cols: usize, rows: usize) -> Self {
        let grid = vec![vec![Cell::blank(); cols]; rows];
        let alt_grid = vec![vec![Cell::blank(); cols]; rows];
        Self {
            grid,
            alt_grid,
            scrollback: VecDeque::new(),
            cursor: (0, 0),
            saved_cursor: (0, 0),
            scroll_region: (0, rows.saturating_sub(1)),
            in_alt_screen: false,
            rows,
            cols,
            current_bold: false,
            current_fg: None,
            current_bg: None,
            parser: vte::Parser::new(),
        }
    }

    /// PTY 出力を処理してグリッドを更新する
    pub fn feed(&mut self, bytes: &[u8]) {
        // self.parser を同時に borrow できないため、swap で取り出す
        let mut parser = mem::replace(&mut self.parser, vte::Parser::new());
        {
            let mut handler = ScreenPerformHandler { screen: self };
            for &byte in bytes {
                parser.advance(&mut handler, byte);
            }
        }
        self.parser = parser;
    }

    /// 現在のスクリーングリッド（通常 or alt）の行テキストを返す
    pub fn screen_line(&self, row: usize) -> String {
        let grid = if self.in_alt_screen { &self.alt_grid } else { &self.grid };
        if row >= grid.len() {
            return String::new();
        }
        let line: String = grid[row].iter().map(|c| c.ch).collect();
        line.trim_end().to_string()
    }

    /// スクロールバック + スクリーングリッドを結合した行テキストを返す
    #[allow(dead_code)]
    pub fn all_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.scrollback.iter().cloned().collect();
        for row in 0..self.rows {
            lines.push(self.screen_line(row));
        }
        lines
    }

    /// ターミナルサイズを変更する（PTY resize 時に呼ぶ）
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;

        // グリッドを新サイズに調整
        self.grid.resize(rows, vec![Cell::blank(); cols]);
        for row in &mut self.grid {
            row.resize(cols, Cell::blank());
        }
        self.alt_grid.resize(rows, vec![Cell::blank(); cols]);
        for row in &mut self.alt_grid {
            row.resize(cols, Cell::blank());
        }

        // スクロール領域をリセット
        self.scroll_region = (0, rows.saturating_sub(1));
        // カーソルをクランプ
        self.cursor = (
            self.cursor.0.min(cols.saturating_sub(1)),
            self.cursor.1.min(rows.saturating_sub(1)),
        );
    }

    /// アクティブなグリッドへの可変参照
    fn active_grid_mut(&mut self) -> &mut Vec<Vec<Cell>> {
        if self.in_alt_screen {
            &mut self.alt_grid
        } else {
            &mut self.grid
        }
    }

    /// アクティブなグリッドへの参照（テストおよび将来の用途向け）
    #[allow(dead_code)]
    fn active_grid(&self) -> &Vec<Vec<Cell>> {
        if self.in_alt_screen {
            &self.alt_grid
        } else {
            &self.grid
        }
    }

    /// スクロール領域内で n 行上スクロールする
    fn scroll_up(&mut self, n: usize) {
        let (top, bot) = self.scroll_region;
        let bot = bot.min(self.rows.saturating_sub(1));
        for _ in 0..n {
            if top == 0 && !self.in_alt_screen {
                let line_text: String = self.grid[top].iter().map(|c| c.ch).collect();
                let trimmed = line_text.trim_end().to_string();
                self.scrollback.push_back(trimmed);
                if self.scrollback.len() > MAX_SCROLLBACK {
                    self.scrollback.pop_front();
                }
            }
            let active = self.active_grid_mut();
            active[top..=bot].rotate_left(1);
            let cols = active[0].len();
            active[bot] = vec![Cell::blank(); cols];
        }
    }

    /// スクロール領域内で n 行下スクロールする（行を下にシフト）
    fn scroll_down(&mut self, n: usize) {
        let (top, bot) = self.scroll_region;
        let bot = bot.min(self.rows.saturating_sub(1));
        let active = self.active_grid_mut();
        let cols = active[0].len();
        for _ in 0..n {
            active[top..=bot].rotate_right(1);
            active[top] = vec![Cell::blank(); cols];
        }
    }
}

/// vte::Perform を実装する内部ハンドラ
struct ScreenPerformHandler<'a> {
    screen: &'a mut VirtualScreen,
}

impl vte::Perform for ScreenPerformHandler<'_> {
    fn print(&mut self, c: char) {
        let (col, row) = self.screen.cursor;
        let cols = self.screen.cols;
        let rows = self.screen.rows;

        if row < rows && col < cols {
            let cell = Cell {
                ch: c,
                bold: self.screen.current_bold,
                fg: self.screen.current_fg,
                bg: self.screen.current_bg,
            };
            let active = self.screen.active_grid_mut();
            active[row][col] = cell;
        }

        // カーソルを右に進める（折り返しは簡易対応）
        let new_col = col + 1;
        if new_col >= cols {
            self.screen.cursor = (cols.saturating_sub(1), row);
        } else {
            self.screen.cursor = (new_col, row);
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // LF: カーソルを1行下に進める。スクロール領域下端なら scroll_up
            0x0a => {
                let (col, row) = self.screen.cursor;
                let (_, bot) = self.screen.scroll_region;
                let bot = bot.min(self.screen.rows.saturating_sub(1));
                if row >= bot {
                    self.screen.scroll_up(1);
                    // カーソル行は bot のまま
                    self.screen.cursor = (col, bot);
                } else {
                    self.screen.cursor = (col, (row + 1).min(self.screen.rows.saturating_sub(1)));
                }
            }
            // CR: col を 0 にする
            0x0d => {
                self.screen.cursor.0 = 0;
            }
            // BS: col を 1 つ戻す
            0x08 => {
                self.screen.cursor.0 = self.screen.cursor.0.saturating_sub(1);
            }
            // BEL: 無視
            0x07 => {}
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        // パラメータを Vec<u16> に変換するヘルパー
        let ps: Vec<u16> = params.iter().map(|sub| sub.first().copied().unwrap_or(0)).collect();
        let p0 = ps.first().copied().unwrap_or(0) as usize;
        let p1 = ps.get(1).copied().unwrap_or(0) as usize;

        // Private mode (? prefix)
        if intermediates == b"?" {
            let n = p0 as u16;
            match (n, action) {
                // smcup: alternate screen に切替え
                (1049 | 1047 | 47, 'h') => {
                    self.screen.in_alt_screen = true;
                    // alt grid をクリア
                    let cols = self.screen.cols;
                    let rows = self.screen.rows;
                    self.screen.alt_grid = vec![vec![Cell::blank(); cols]; rows];
                    self.screen.cursor = (0, 0);
                }
                // rmcup: main screen に復帰
                (1049 | 1047 | 47, 'l') => {
                    self.screen.in_alt_screen = false;
                }
                // カーソル表示/非表示・その他: 無視
                _ => {}
            }
            return;
        }

        match action {
            // CUU: カーソルを n 行上
            'A' => {
                let n = p0.max(1);
                let (col, row) = self.screen.cursor;
                self.screen.cursor = (col, row.saturating_sub(n));
            }
            // CUD: カーソルを n 行下
            'B' => {
                let n = p0.max(1);
                let (col, row) = self.screen.cursor;
                self.screen.cursor = (col, (row + n).min(self.screen.rows.saturating_sub(1)));
            }
            // CUF: カーソルを n 列右
            'C' => {
                let n = p0.max(1);
                let (col, row) = self.screen.cursor;
                self.screen.cursor = ((col + n).min(self.screen.cols.saturating_sub(1)), row);
            }
            // CUB: カーソルを n 列左
            'D' => {
                let n = p0.max(1);
                let (col, row) = self.screen.cursor;
                self.screen.cursor = (col.saturating_sub(n), row);
            }
            // CUP/HVP: カーソル移動 (row, col), 1-indexed
            'H' | 'f' => {
                let row = p0.saturating_sub(1);
                let col = p1.saturating_sub(1);
                self.screen.cursor = (
                    col.min(self.screen.cols.saturating_sub(1)),
                    row.min(self.screen.rows.saturating_sub(1)),
                );
            }
            // ED: 画面消去
            'J' => {
                let rows = self.screen.rows;
                let (cur_col, cur_row) = self.screen.cursor;
                match p0 {
                    0 => {
                        // カーソル以降を消去
                        let active = self.screen.active_grid_mut();
                        active[cur_row][cur_col..].fill(Cell::blank());
                        for row in &mut active[(cur_row + 1)..rows] {
                            row.fill(Cell::blank());
                        }
                    }
                    1 => {
                        // カーソル以前を消去
                        let active = self.screen.active_grid_mut();
                        for row in &mut active[..cur_row] {
                            row.fill(Cell::blank());
                        }
                        active[cur_row][..=cur_col].fill(Cell::blank());
                    }
                    2 | 3 => {
                        // 全体消去
                        let active = self.screen.active_grid_mut();
                        for row in active.iter_mut() {
                            row.fill(Cell::blank());
                        }
                    }
                    _ => {}
                }
            }
            // EL: 行消去
            'K' => {
                let (cur_col, cur_row) = self.screen.cursor;
                match p0 {
                    0 => {
                        // カーソルから行末を消去
                        let active = self.screen.active_grid_mut();
                        active[cur_row][cur_col..].fill(Cell::blank());
                    }
                    1 => {
                        // 行頭からカーソルを消去
                        let active = self.screen.active_grid_mut();
                        active[cur_row][..=cur_col].fill(Cell::blank());
                    }
                    2 => {
                        // 行全体を消去
                        let active = self.screen.active_grid_mut();
                        active[cur_row].fill(Cell::blank());
                    }
                    _ => {}
                }
            }
            // DECSTBM: スクロール領域設定
            'r' => {
                let top = if p0 == 0 { 1 } else { p0 };
                let bot = if p1 == 0 { self.screen.rows } else { p1 };
                self.screen.scroll_region = (
                    (top - 1).min(self.screen.rows.saturating_sub(1)),
                    (bot - 1).min(self.screen.rows.saturating_sub(1)),
                );
            }
            // SU: n 行スクロールアップ
            'S' => {
                let n = p0.max(1);
                self.screen.scroll_up(n);
            }
            // SD: n 行スクロールダウン
            'T' => {
                let n = p0.max(1);
                self.screen.scroll_down(n);
            }
            // IL: n 行挿入
            'L' => {
                let n = p0.max(1);
                self.screen.scroll_down(n);
            }
            // DL: n 行削除
            'M' => {
                let n = p0.max(1);
                self.screen.scroll_up(n);
            }
            // SGR: 文字属性
            'm' => {
                self.handle_sgr(&ps);
            }
            // その他は無視
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            // ESC 7: カーソル保存
            b'7' => {
                self.screen.saved_cursor = self.screen.cursor;
            }
            // ESC 8: カーソル復元
            b'8' => {
                self.screen.cursor = self.screen.saved_cursor;
            }
            _ => {}
        }
    }
}

impl ScreenPerformHandler<'_> {
    fn handle_sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            self.reset_attrs();
            return;
        }

        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => self.reset_attrs(),
                1 => self.screen.current_bold = true,
                22 => self.screen.current_bold = false,
                30..=37 => self.screen.current_fg = Some(Color::Ansi(params[i] as u8 - 30)),
                38 => {
                    if params.get(i + 1) == Some(&5)
                        && let Some(&n) = params.get(i + 2)
                    {
                        self.screen.current_fg = Some(Color::Ansi(n as u8));
                        i += 2;
                    } else if params.get(i + 1) == Some(&2)
                        && let (Some(&r), Some(&g), Some(&b)) = (
                            params.get(i + 2),
                            params.get(i + 3),
                            params.get(i + 4),
                        )
                    {
                        self.screen.current_fg = Some(Color::Rgb(r as u8, g as u8, b as u8));
                        i += 4;
                    }
                }
                39 => self.screen.current_fg = None,
                40..=47 => self.screen.current_bg = Some(Color::Ansi(params[i] as u8 - 40)),
                48 => {
                    if params.get(i + 1) == Some(&5)
                        && let Some(&n) = params.get(i + 2)
                    {
                        self.screen.current_bg = Some(Color::Ansi(n as u8));
                        i += 2;
                    } else if params.get(i + 1) == Some(&2)
                        && let (Some(&r), Some(&g), Some(&b)) = (
                            params.get(i + 2),
                            params.get(i + 3),
                            params.get(i + 4),
                        )
                    {
                        self.screen.current_bg = Some(Color::Rgb(r as u8, g as u8, b as u8));
                        i += 4;
                    }
                }
                49 => self.screen.current_bg = None,
                _ => {}
            }
            i += 1;
        }
    }

    fn reset_attrs(&mut self) {
        self.screen.current_bold = false;
        self.screen.current_fg = None;
        self.screen.current_bg = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_screen(cols: usize, rows: usize) -> VirtualScreen {
        VirtualScreen::new(cols, rows)
    }

    #[test]
    fn test_print_writes_char_to_grid() {
        let mut s = make_screen(10, 5);
        s.feed(b"A");
        assert_eq!(s.active_grid()[0][0].ch, 'A');
    }

    #[test]
    fn test_lf_advances_cursor_row() {
        let mut s = make_screen(10, 5);
        s.feed(b"A\nB");
        // cursor を行0→行1→行2と進める
        // A を書いた後 col=1, row=0
        // LF で row=1
        // B を col=0 に書く（CR がない場合は col=1 のまま）
        // ここでは col=1 のままなので grid[1][1] に B が書かれる
        assert_eq!(s.active_grid()[1][1].ch, 'B');
    }

    #[test]
    fn test_lf_at_bottom_scrolls_out() {
        // 5行のスクリーン。最下行で LF → scrollback に行が追加される
        let mut s = make_screen(10, 5);
        // 5行分書き込んでから追加の LF
        s.feed(b"line1\r\nline2\r\nline3\r\nline4\r\nline5\r\n");
        // scrollback に "line1" が追加されているはず
        assert!(!s.scrollback.is_empty(), "scrollback should have entries");
        assert_eq!(s.scrollback.front().map(|s| s.as_str()), Some("line1"));
    }

    #[test]
    fn test_cr_resets_cursor_col() {
        let mut s = make_screen(10, 5);
        s.feed(b"ABC\rX");
        // CR で col=0 に戻り、X を col=0 に書く
        assert_eq!(s.active_grid()[0][0].ch, 'X');
    }

    #[test]
    fn test_csi_h_moves_cursor() {
        let mut s = make_screen(10, 5);
        // CSI 3;5H → row=2, col=4
        s.feed(b"\x1b[3;5H");
        assert_eq!(s.cursor, (4, 2));
    }

    #[test]
    fn test_csi_h_no_params_goes_home() {
        let mut s = make_screen(10, 5);
        s.feed(b"ABC");
        s.feed(b"\x1b[H");
        assert_eq!(s.cursor, (0, 0));
    }

    #[test]
    fn test_csi_j2_clears_screen() {
        let mut s = make_screen(10, 5);
        s.feed(b"HELLO");
        s.feed(b"\x1b[2J");
        // 全セルが blank になる
        for row in 0..5 {
            for col in 0..10 {
                assert_eq!(s.active_grid()[row][col].ch, ' ', "cell [{row}][{col}] should be blank");
            }
        }
    }

    #[test]
    fn test_csi_k0_clears_to_eol() {
        let mut s = make_screen(10, 5);
        s.feed(b"HELLO");
        // カーソルを col=2 に移動して EL 0
        s.feed(b"\x1b[1;3H\x1b[K");
        // col 0,1 は "HE"、col 2-9 は blank
        assert_eq!(s.active_grid()[0][0].ch, 'H');
        assert_eq!(s.active_grid()[0][1].ch, 'E');
        for col in 2..10 {
            assert_eq!(s.active_grid()[0][col].ch, ' ', "col {col} should be blank");
        }
    }

    #[test]
    fn test_sgr_sets_bold() {
        let mut s = make_screen(10, 5);
        s.feed(b"\x1b[1mA");
        assert!(s.active_grid()[0][0].bold, "A should be bold");
    }

    #[test]
    fn test_sgr_reset() {
        let mut s = make_screen(10, 5);
        s.feed(b"\x1b[1m");
        assert!(s.current_bold);
        s.feed(b"\x1b[0m");
        assert!(!s.current_bold);
        assert!(s.current_fg.is_none());
        assert!(s.current_bg.is_none());
    }

    #[test]
    fn test_sgr_fg_color() {
        let mut s = make_screen(10, 5);
        s.feed(b"\x1b[31m");
        assert_eq!(s.current_fg, Some(Color::Ansi(1)));
    }

    #[test]
    fn test_sgr_reset_no_params() {
        let mut s = make_screen(10, 5);
        s.feed(b"\x1b[1m\x1b[31m");
        s.feed(b"\x1b[m");
        assert!(!s.current_bold);
        assert!(s.current_fg.is_none());
    }

    #[test]
    fn test_scroll_region_r() {
        let mut s = make_screen(10, 10);
        // CSI 2;8r → top=1, bot=7 (0-indexed)
        s.feed(b"\x1b[2;8r");
        assert_eq!(s.scroll_region, (1, 7));
    }

    #[test]
    fn test_scroll_with_region_does_not_scrollback() {
        // スクロール領域が 0 行目を含まないときは scrollback に追加しない
        let mut s = make_screen(10, 10);
        // スクロール領域を row 2-9 に設定
        s.feed(b"\x1b[3;10r");
        // row 2 に移動して何行か書く
        s.feed(b"\x1b[3;1H");
        for _ in 0..20 {
            s.feed(b"X\r\n");
        }
        assert!(s.scrollback.is_empty(), "scrollback should be empty when scroll region does not include row 0");
    }

    #[test]
    fn test_screen_line_returns_row_text() {
        let mut s = make_screen(10, 5);
        s.feed(b"hello");
        assert_eq!(s.screen_line(0), "hello");
    }

    #[test]
    fn test_screen_line_trims_trailing_spaces() {
        let mut s = make_screen(10, 5);
        s.feed(b"hi");
        let line = s.screen_line(0);
        assert_eq!(line, "hi");
        assert!(!line.ends_with(' '), "should trim trailing spaces");
    }

    #[test]
    fn test_all_lines_combines_scrollback_and_screen() {
        let mut s = make_screen(10, 3);
        // 3行のスクリーンに 4 行書くと 1 行がスクロールアウト
        s.feed(b"line1\r\nline2\r\nline3\r\nline4");
        let all = s.all_lines();
        // scrollback に "line1"、screen に "line2","line3","line4"
        assert!(all.len() >= 4, "all_lines should have at least 4 lines: {:?}", all);
        assert_eq!(all[0], "line1", "first scrollback line should be 'line1'");
    }

    #[test]
    fn test_resize_expands_grid() {
        let mut s = make_screen(10, 5);
        s.resize(20, 10);
        assert_eq!(s.cols, 20);
        assert_eq!(s.rows, 10);
        assert_eq!(s.grid.len(), 10);
        assert_eq!(s.grid[0].len(), 20);
    }

    #[test]
    fn test_smcup_switches_to_alt_grid() {
        let mut s = make_screen(10, 5);
        // main grid に A を書く
        s.feed(b"A");
        assert_eq!(s.grid[0][0].ch, 'A');
        // smcup
        s.feed(b"\x1b[?1049h");
        assert!(s.in_alt_screen);
        // alt grid に B を書く
        s.feed(b"B");
        // alt grid に B が書かれる
        assert_eq!(s.alt_grid[0][0].ch, 'B');
        // main grid の A は保持されている
        assert_eq!(s.grid[0][0].ch, 'A');
    }

    #[test]
    fn test_rmcup_restores_main_grid() {
        let mut s = make_screen(10, 5);
        s.feed(b"A");
        s.feed(b"\x1b[?1049h");
        s.feed(b"B");
        // rmcup で main grid に戻る
        s.feed(b"\x1b[?1049l");
        assert!(!s.in_alt_screen);
        // main grid の A は保持
        assert_eq!(s.grid[0][0].ch, 'A');
    }

    #[test]
    fn test_esc_save_restore_cursor() {
        let mut s = make_screen(10, 5);
        s.feed(b"\x1b[3;5H"); // カーソルを (row=2, col=4) に移動
        s.feed(b"\x1b7");     // カーソル保存
        s.feed(b"\x1b[H");    // ホームへ移動
        assert_eq!(s.cursor, (0, 0));
        s.feed(b"\x1b8");     // カーソル復元
        assert_eq!(s.cursor, (4, 2));
    }

    #[test]
    fn test_cursor_movement_a_b_c_d() {
        let mut s = make_screen(20, 10);
        s.feed(b"\x1b[5;5H"); // row=4, col=4
        s.feed(b"\x1b[2A");   // 2行上 → row=2
        assert_eq!(s.cursor.1, 2);
        s.feed(b"\x1b[3B");   // 3行下 → row=5
        assert_eq!(s.cursor.1, 5);
        s.feed(b"\x1b[4C");   // 4列右 → col=8
        assert_eq!(s.cursor.0, 8);
        s.feed(b"\x1b[2D");   // 2列左 → col=6
        assert_eq!(s.cursor.0, 6);
    }
}
