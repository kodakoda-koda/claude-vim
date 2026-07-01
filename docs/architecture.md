# Architecture

## Overview

claude-vim は Claude Code CLI を PTY 経由でサブプロセスとして起動し、stdin/stdout を仲介するラッパー。
4つのモード（Normal/Insert/Cursor/Visual）を持ち、vim スタイルのモーダル操作を提供する。

```
┌─────────────────────────────────────────────────┐
│  claude-vim (wrapper process)                   │
│                                                 │
│  Insert mode:                                   │
│    stdin (raw bytes) ──→ PTY(claude)            │
│    ※Esc 単独（50ms timeout）→ Normal mode      │
│    ※Enter → Shift+Enter に変換（改行）         │
│                                                 │
│  Normal mode:                                   │
│    j/k/Ctrl+d/u/f/b ──→ SGR マウスイベント→PTY │
│    数値プレフィックス対応（10j = 10回分）       │
│    gg ──→ Ctrl+Home→PTY、G ──→ Ctrl+End→PTY   │
│    c ──→ Cursor mode 切替                       │
│    printable 文字 ──→ 無視                      │
│    control/esc seq ──→ PTY へ透過               │
│                                                 │
│  Cursor mode:                                   │
│    hjkl ──→ カーソル移動（1文字ハイライト）     │
│    v ──→ Visual mode（選択開始）               │
│                                                 │
│  Visual mode:                                   │
│    hjkl ──→ 選択範囲拡縮（文字単位ハイライト） │
│    y ──→ クリップボードにコピー → Normal       │
│                                                 │
│  VirtualScreen (vte):                           │
│    PTY output ──→ feed ──→ grid + scrollback    │
│    Cursor/Visual mode がテキスト取得に使用      │
│                                                 │
│  Statusline:                                    │
│    最下行に NORMAL/INSERT/CURSOR/VISUAL + info  │
│    PTY には rows-1 を報告                       │
│                                                 │
│  PTY output ──→ filter_smcup ──→ stdout         │
│                ──→ VirtualScreen.feed()          │
└──────────────┬──────────────────────────────────┘
               │ PTY
┌──────────────▼──────────────────────────────────┐
│  claude (subprocess)                            │
└─────────────────────────────────────────────────┘
```

## モードフロー

```
           ┌────────────────────────────┐
           │                            │
Insert ←(i)── Normal ──(c)──→ Cursor ──(v)──→ Visual
  │              ↑              │ (Esc)         │ (Esc/y)
  └──(Esc)───────┘              └───→ Normal ←──┘
```

---

## スクロール方式

### 採用: SGR マウスホイールイベント

Claude Code は SGR マウスモード（`\x1b[?1006h`）を有効化している。
Normal mode のスクロールキーを SGR マウスホイールイベントに変換して PTY に直接送信する。
Claude Code がこれを受け取り、入力欄を固定したまま会話履歴だけをスクロールする。

```
j/k      →  SGR マウスホイールイベント N回（\x1b[<64;col;rowM / \x1b[<65;col;rowM）
Ctrl+d/u →  rows/2/LINES_PER_EVENT 回（ウィンドウサイズ依存）
Ctrl+f/b →  rows/LINES_PER_EVENT 回（ウィンドウサイズ依存）
gg       →  Ctrl+Home（\x1b[1;5H）
G        →  Ctrl+End（\x1b[1;5F）
```

数値プレフィックス（`10j` 等）は N 倍のイベントを一括送信する。
加速防止のため、スクロールイベント間に 50ms のスロットルを設けている。

### 不採用: Kitty scroll-window

当初は `kitty @ scroll-window` コマンドを使う方針だった。
しかし Kitty の scrollback scroll と Claude Code のアプリ内スクロールは根本的に別物であり、
Kitty scroll では TUI chrome が混入して実用にならなかった。

---

## VirtualScreen

`vte` crate で PTY 出力をパースし、ターミナル画面の内容をセル単位で保持する。

- PTY reader スレッドで `filter_smcup()` 適用後のバイト列を `VirtualScreen.feed()` に渡す
- `Arc<Mutex<VirtualScreen>>` で PTY reader スレッドと App で共有
- Cursor/Visual mode が `screen_line(row)` でテキストを取得し、ハイライト描画や yank に使用

### Claude Code の再描画パターン（実測）

Claude Code はスクロール時に**部分更新**する（全画面再描画ではない）。
24行中 2-3 行のみ更新されることが確認されている。
VirtualScreen の grid は部分更新後も正しくターミナル表示と一致する。

### Cursor/Visual mode のハイライト

overlay 方式を採用。VirtualScreen からオリジナル行テキストを読み、反転色（SGR 7m）で上書き描画する。
カーソル移動や選択範囲変更時は、前のハイライト行をオリジナル内容で復元してから新しいハイライトを描画する。
Normal mode に戻るときも同様に復元する。

---

## Claude Code の挙動（実測・v2.1.170）

### 起動時に送るエスケープシーケンス（順番通り）

```
\x1b7              DEC カーソル保存
\x1b[r             スクロール領域リセット
\x1b8              DEC カーソル復元
\x1b[?25h          カーソル表示
\x1b[?1049h        ★ alternate screen 入場（SMCUP）← フィルタ
\x1b[2J            画面クリア
\x1b[H             カーソルをホームへ
\x1b[<u            Kitty keyboard protocol 無効化
\x1b[>1u           ★ Kitty keyboard protocol level 1 有効化
\x1b[>4;2m         キー修飾子エンコーディング設定
\x1b[?1000h        マウストラッキング（X10）
\x1b[?1002h        マウストラッキング（ボタンイベント）
\x1b[?1003h        マウストラッキング（全イベント）
\x1b[?1006h        マウストラッキング（SGR モード）
\x1b[?25l          カーソル非表示
\x1b[?2004h        bracketed paste 有効化
\x1b[?1004h        フォーカストラッキング
\x1b[?2031h        カラースキームレポーティング
\x1b[?2026h        ★ synchronized output 開始
```

### Kitty keyboard protocol の影響

Claude Code が `\x1b[>1u` を送ると、対応ターミナルは拡張キーシーケンスを使い始める。
例: Esc → `\x1b[27u`、Ctrl+d → `\x1b[100;5u`、Shift+Enter → `\x1b[13;2u`

claude-vim は両方のフォーマット（レガシーと Kitty kbd protocol）を認識する。

---

## 設計決定の経緯

### Insert mode の raw byte passthrough

crossterm のイベントモデルではなく、stdin の生バイトをそのまま PTY に流す方式を採用。
Kitty keyboard protocol の拡張シーケンスを正確に転送するため。

### Esc の扱い

Esc が単独で来た場合（後続バイトなし、50ms タイムアウト）のみ Normal mode に切替。
Claude Code には Esc を送らない。ダイアログの誤キャンセルを防止。

### Normal mode で printable 文字を無視

Normal mode で `a` や `f` が Claude Code の入力欄に入力されてしまう問題が発覚。
printable 文字（0x20-0x7e）は無視し、control chars と escape sequences のみ透過に変更。

### Cursor mode の導入

当初は Normal mode から直接 Visual mode に入る設計だったが、
カーソル位置を指定してから選択を開始できないという問題があった。
Cursor mode を中間に挟むことで、hjkl でカーソルを移動してから v で Visual に入るフローを実現。
