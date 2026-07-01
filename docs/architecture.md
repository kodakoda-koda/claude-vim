# Architecture

## Overview

cv は Claude Code CLI を PTY 経由でサブプロセスとして起動し、stdin/stdout を仲介するラッパー。
vim のモーダル操作（Normal/Insert）を追加し、Normal mode ではスクロール、Insert mode ではテキスト入力を行う。

```
┌─────────────────────────────────────────────────┐
│  cv (wrapper process)                           │
│                                                 │
│  Insert mode:                                   │
│    stdin (raw bytes) ──→ PTY(claude)            │
│    ※Esc 単独（50ms timeout）→ Normal mode      │
│    ※Enter → Shift+Enter に変換（改行）         │
│    PTY(claude) output ──→ filter ──→ stdout     │
│                                                 │
│  Normal mode:                                   │
│    j/k/Ctrl+d/u/f/b ──→ SGR マウスイベント→PTY │
│    gg ──→ Ctrl+Home→PTY、G ──→ Ctrl+End→PTY   │
│    i ──→ Insert mode 切替                       │
│    printable 文字 ──→ 無視                      │
│    control/esc seq ──→ PTY へ透過               │
│                                                 │
│  Statusline:                                    │
│    最下行に NORMAL/INSERT + branch + version    │
│    PTY には rows-1 を報告                       │
└──────────────┬──────────────────────────────────┘
               │ PTY
┌──────────────▼──────────────────────────────────┐
│  claude (subprocess)                            │
└─────────────────────────────────────────────────┘
```

## スクロール方式

### 採用: SGR マウスホイールイベント

Claude Code は SGR マウスモード（`\x1b[?1006h`）を有効化している。
cv は Normal mode のスクロールキーを SGR マウスホイールイベントに変換して PTY に直接送信する。
Claude Code がこれを受け取り、入力欄を固定したまま会話履歴だけをスクロールする。

```
j/k      →  SGR マウスホイールイベント 1回（\x1b[<64;col;rowM / \x1b[<65;col;rowM）
Ctrl+d/u →  rows/2/LINES_PER_EVENT 回（ウィンドウサイズ依存）
Ctrl+f/b →  rows/LINES_PER_EVENT 回（ウィンドウサイズ依存）
gg       →  Ctrl+Home（\x1b[1;5H）
G        →  Ctrl+End（\x1b[1;5F）
```

加速防止のため、スクロールイベント間に 50ms のスロットルを設けている。

### 不採用: Kitty scroll-window

当初は `kitty @ scroll-window` コマンドを使う方針だった（v0.1 の最初の設計）。
しかし Kitty の scrollback scroll と Claude Code のアプリ内スクロールは根本的に別物であり、
Kitty scroll では TUI chrome が混入して実用にならなかった。
また Kitty 依存が不要になるという副次的なメリットもあった。

### 今後: vte 自前エミュレーション（v0.2）

`vte` crate で VirtualScreen を実装し、Claude Code の PTY 出力を自前でパース・蓄積する。
PoC は完了済み（scrollback に自然にスクロールアウトした行を正確にキャプチャできることを確認）。

---

## Claude Code の挙動（実測・v2.1.170）

### 起動時に送るエスケープシーケンス（順番通り）

```
\x1b7              DEC カーソル保存
\x1b[r             スクロール領域リセット
\x1b8              DEC カーソル復元
\x1b[?25h          カーソル表示
\x1b[?1049h        ★ alternate screen 入場（SMCUP）← cv でフィルタ
\x1b[2J            画面クリア
\x1b[H             カーソルをホームへ
\x1b[<u            Kitty keyboard protocol 無効化（前回セッション cleanup）
\x1b[>1u           ★ Kitty keyboard protocol level 1 有効化
\x1b[>4;2m         キー修飾子エンコーディング設定
\x1b[?1000h        マウストラッキング（X10）
\x1b[?1002h        マウストラッキング（ボタンイベント）
\x1b[?1003h        マウストラッキング（全イベント）
\x1b[?1006h        マウストラッキング（SGR モード）
\x1b[?25l          カーソル非表示
\x1b[?2004h        bracketed paste 有効化
\x1b[?1004h        フォーカストラッキング
\x1b[?2031h        カラースキームレポーティング（dark/light 変更通知）
\x1b[?2026h        ★ synchronized output 開始（フレーム境界マーカー）
```

### Kitty keyboard protocol の影響

Claude Code が `\x1b[>1u` を送ると、対応ターミナルは拡張キーシーケンスを使い始める。
例: Esc → `\x1b[27u`、Ctrl+d → `\x1b[100;5u`、Shift+Enter → `\x1b[13;2u`

cv は両方のフォーマット（レガシーと Kitty kbd protocol）を認識する。

### smcup フィルタ

cv がフィルタして除去するシーケンス:

- `\x1b[?1049h` / `\x1b[?1049l`（smcup/rmcup）
- `\x1b[?1047h` / `\x1b[?1047l`（旧形式）
- `\x1b[?47h` / `\x1b[?47l`（さらに旧形式）
- `\x1b[3J`（スクロールバック消去）

### Claude Code の全インタラクティブ状態と cv の対応

| 状態 | キー操作 | cv の対応 |
|------|---------|-----------|
| 通常会話（入力中） | Enter=送信, Ctrl-C=中断 | Normal: Enter 透過で送信。Insert: Enter→改行 |
| /resume ピッカー | j/k=選択, Enter=決定, Esc=キャンセル | Insert で操作（j/k は Normal だとスクロール） |
| /model ピッカー | 同上 | 同上 |
| パーミッション確認 | y/n or Enter/Esc | Normal: Enter/Ctrl+C 透過。Insert: raw passthrough |
| パーミッションモード切替 | Shift+Tab | 両モードで透過（Esc seq として透過） |
| Claude 応答ストリーミング中 | Ctrl-C=中断 | 両モードで透過 |

---

## 設計決定の経緯

### Insert mode の raw byte passthrough

crossterm のイベントモデルではなく、stdin の生バイトをそのまま PTY に流す方式を採用。
Kitty keyboard protocol の拡張シーケンス（`\x1b[13;2u` 等）を正確に転送するため。

### Esc の扱い

Esc が単独で来た場合（後続バイトなし、50ms タイムアウト）のみ Normal mode に切替。
Claude Code には Esc を送らない。これによりダイアログの誤キャンセルを防止。

### Normal mode で printable 文字を無視

当初は「その他すべて → PTY に透過」としていたが、
Normal mode で `a` や `f` が Claude Code の入力欄に入力されてしまう問題が発覚。
printable 文字（0x20-0x7e）は無視し、control chars と escape sequences のみ透過に変更。

### scrollback 手法の比較（検証済み）

| 手法 | 品質 | 端末依存 | 採用 |
|------|------|---------|------|
| SGR マウスイベント | ◯ Claude Code 内部スクロール | 不要 | **v0.1 採用** |
| smcup filter + kitty scroll-window | △ TUI chrome 混入 | Kitty のみ | 不採用 |
| kitty @ get-text | ◯ | Kitty のみ | 未実装 |
| vte 自前エミュレーション | ◎ 完全クリーン | 不要 | **v0.2 予定** |
