# CLAUDE.md

## プロジェクト概要

`cv` は Claude Code に vim のモーダル操作を追加するスタンドアロンの PTY ラッパー。Neovim やプラグインには依存しない単一バイナリ。

```
Insert mode  →  Claude Code への完全パススルー（通常の Claude Code 操作）
Esc          →  Normal mode（Kitty の scroll API で会話履歴を vim キーでスクロール）
i            →  Insert mode に戻る
```

## ロードマップ

| バージョン | スコープ |
|---|---|
| **v0.1** | Kitty 限定。基本的な Insert/Normal mode + vim スクロール |
| **v0.2** | tmux・WezTerm など他ターミナル対応、Visual mode + yank |

## 確定した設計方針

### v0.1 は Kitty 専用

他ターミナルへの対応は v0.2 以降。将来的には起動時にターミナルを検出して以下のように分岐する想定：

```
$KITTY_WINDOW_ID あり  → kitty @ scroll-window（v0.1 実装済み）
tmux セッション内       → tmux copy-mode（v0.2）
WezTerm               → wezterm cli（v0.2）
それ以外              → 自前 alternate screen viewer（v0.2）
```

### Normal mode のスクロールは Kitty remote control を使う

**最重要の設計決定。**

当初は Normal mode で alternate screen に切り替えて自前の scrollback viewer を表示する案を検討したが、**「見た目は Claude Code のままにしたい」** という要件のため、Kitty の remote control API を使うアプローチに確定した。

```bash
kitty @ scroll-window up 5 lines    # Ctrl+U
kitty @ scroll-window down 5 lines  # Ctrl+D
```

- Normal mode に入っても画面は切り替わらない
- Claude Code の TUI がそのまま見える状態でスクロールできる
- ユーザーは Kitty を使用しているため、Kitty 依存は許容される

**前提条件:** ユーザーの `~/.config/kitty/kitty.conf` に以下が必要：

```
allow_remote_control yes
listen_on unix:/tmp/mykitty  # または socket
```

### Visual mode は v0.2 以降に延期

Kitty scroll API アプローチでは Visual mode によるテキスト選択の実装が難しい（alternate screen を使わないため）。v0.1 スコープから除外。

## アーキテクチャ

```
┌─────────────────────────────────────────────────┐
│  cv (wrapper process)                           │
│                                                 │
│  Insert mode:                                   │
│    stdin ──→ PTY(claude)                        │
│    PTY(claude) ──→ stdout                       │
│                                                 │
│  Normal mode:                                   │
│    vim keys ──→ kitty @ scroll-window           │
│    Esc/i/etc ──→ mode 切替                      │
│    その他のキー ──→ 無視（PTY には送らない）    │
└──────────────┬──────────────────────────────────┘
               │ PTY
┌──────────────▼──────────────────────────────────┐
│  claude (subprocess)                            │
└─────────────────────────────────────────────────┘
```

**モード切替の仕組み:**
- `Esc` → キーのインターセプト開始、以降の入力を vim キーとして解釈
- `i` → インターセプト解除、Insert mode 復帰（Claude Code への passthrough 再開）
- Claude Code のプロセスは両モードで常に動き続ける

## ファイル構成（実装予定）

```
src/
├── main.rs    - エントリポイント。ターミナルサイズ取得、raw mode 設定
├── app.rs     - メインイベントループ。モード管理、イベントの振り分け
├── pty.rs     - portable-pty で claude をサブプロセス起動、reader/writer スレッド
├── kitty.rs   - kitty @ scroll-window コマンドのラッパー
└── input.rs   - Normal mode のキーバインド処理
```

`term.rs` と `ui.rs` は alternate screen viewer アプローチを使わないため不要になった。

## 主要な依存 crate

- `portable-pty` — PTY 管理（WezTerm 製）
- `crossterm` — raw mode、キー入力

`vte` は alternate screen viewer を作らないため不要。

## キーバインド（確定）

| キー | Normal mode での動作 |
|------|---------------------|
| `Esc` | Insert → Normal |
| `i` | Normal → Insert |
| `j` | 1行下スクロール |
| `k` | 1行上スクロール |
| `Ctrl+d` | 半ページ下 |
| `Ctrl+u` | 半ページ上 |
| `Ctrl+f` | 全ページ下 |
| `Ctrl+b` | 全ページ上 |
| `g g` | 最上部へ |
| `G` | 最下部へ（最新出力） |

## Issues との対応

| Issue | 内容 | バージョン | 状態 |
|-------|------|-----------|------|
| #1 | PTY wrapper infrastructure | v0.1 | 未着手 |
| #2 | Insert mode: raw passthrough | v0.1 | 未着手 |
| #3 | Normal mode: Kitty scroll API | v0.1 | 未着手 |
| #4 | Vim navigation keys | v0.1 | 未着手 |
| #5 | Visual mode and yank | v0.2 | 延期 |
| #6 | Multi-terminal support | v0.2 | 延期 |

## 開発ワークフロー

- `feat/issue-N-description` ブランチを切って実装 → PR
- `cargo build` でビルド確認、`cargo clippy` でリント

## ビルド

```bash
cargo build           # debug
cargo build --release
./target/debug/cv     # 実行
```

## 実装時の注意

- Kitty の socket path は環境変数 `$KITTY_LISTEN_ON` から取得できる（`kitty --listen-on` で起動した場合）
- `kitty @` コマンドは Kitty が `allow_remote_control yes` でないと動かない。起動時にチェックしてエラーを出すべき
- Normal mode 中も Claude Code の PTY 出力は流れ続ける（Claude が応答中など）。この出力は Insert mode 復帰時に自動的に表示される
