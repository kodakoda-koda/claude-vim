# CLAUDE.md

## プロジェクト概要

`cv` は Claude Code に vim のモーダル操作を追加するスタンドアロンの PTY ラッパー。Neovim やプラグインには依存しない単一バイナリ。

```
Insert mode  →  Claude Code への完全パススルー（通常の Claude Code 操作）
Esc          →  Normal mode（会話履歴を vim キーでスクロール）
i            →  Insert mode に戻る
```

## アーキテクチャ

```
┌─────────────────────────────────────────┐
│  cv (wrapper process)                   │
│                                         │
│  Insert mode: stdin → PTY → stdout      │
│  Normal mode: alternate screen に       │
│               scrollback viewer を表示  │
└──────────────┬──────────────────────────┘
               │ PTY
┌──────────────▼──────────────────────────┐
│  claude (subprocess)                    │
└─────────────────────────────────────────┘
```

**モード切替の仕組み:**
- `Esc` → PTY 出力のバッファリング開始、alternate screen に scrollback viewer を描画
- `i` → alternate screen を閉じ、Claude Code に SIGWINCH を送って強制再描画、Insert mode 復帰

## ファイル構成

```
src/
├── main.rs   - エントリポイント。ターミナルサイズ取得、raw mode 設定
├── app.rs    - メインイベントループ。モード管理、イベントの振り分け
├── pty.rs    - portable-pty で claude をサブプロセス起動、reader/writer スレッド
├── term.rs   - vte::Perform 実装。PTY 出力をパースして scrollback を蓄積
└── ui.rs     - Normal mode の crossterm レンダリング。vim ナビゲーション
```

## 主要な依存 crate

- `portable-pty` — PTY 管理（WezTerm 製）
- `vte` — ANSI/VT エスケープシーケンスパーサ（Alacritty と同じ）
- `crossterm` — raw mode、キー入力、ANSI 出力

## 開発ワークフロー

- 各機能は GitHub Issue に対応（#1〜#5）
- Issue ブランチ（`feat/issue-N-description`）を切って実装 → PR
- `cargo build` でビルド確認、`cargo clippy` でリント

## ビルド

```bash
cargo build           # debug
cargo build --release # release
./target/debug/cv     # 実行
```

## 実装上の注意

- Claude Code は起動時に alternate screen に切り替える（`\x1b[?1049h`）。Normal mode で我々も alternate screen を使う場合、Claude の PTY 内の alternate screen とは別レイヤーになるため競合しない
- `vte` の Perform 実装では、Claude Code の TUI リドローによるノイズ（スピナー等）が scrollback に混入することがある。v0.1 では許容する
- クリップボードへのコピーは macOS: `pbcopy`、Linux: `xclip` / `wl-copy` を使う
