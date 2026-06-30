# CLAUDE.md

## プロジェクト概要

`cv` は Claude Code に vim のモーダル操作を追加する PTY ラッパー。単一バイナリ、ターミナル非依存。

- Normal mode: スクロール（SGR マウスイベント経由）+ control/esc seq 透過
- Insert mode: テキスト入力（raw passthrough、Enter→改行変換）

## ファイル構成

```
src/
├── main.rs        エントリポイント。raw mode、PTY 起動、SIGWINCH
├── app.rs         イベントループ。モード管理、Esc 検出（50ms timeout）
├── pty.rs         portable-pty で claude 起動、smcup フィルタ
├── scroll.rs      SGR マウスイベントでスクロール（Scroller）
├── input.rs       Normal mode キーマッチング（raw bytes → InputAction）
└── statusline.rs  lualine 風ステータスライン（モード・ブランチ・バージョン）
```

## ビルド・実行

```bash
cargo build
cargo clippy
cargo test
./target/debug/cv
```

## キーバインド

### Insert mode
| キー | 動作 |
|------|------|
| `Esc` / `\x1b[27u` | Normal mode へ |
| `Enter` | 改行（Shift+Enter に変換） |
| その他 | raw passthrough |

### Normal mode
| キー | 動作 |
|------|------|
| `i` | Insert mode へ |
| `j`/`k` | 1行スクロール |
| `Ctrl+d`/`Ctrl+u` | 半ページ（rows/6 イベント） |
| `Ctrl+f`/`Ctrl+b` | 全ページ（rows/3 イベント） |
| `gg` / `G` | 最上部 / 最下部（Ctrl+Home/End） |
| printable 文字 | 無視 |
| control/esc seq | PTY へ透過 |

## Issue 対応

| Issue | 内容 | 状態 |
|-------|------|------|
| #1 | PTY wrapper infrastructure | v0.1 完了 |
| #2 | Insert mode: raw passthrough | v0.1 完了 |
| #3 | Normal mode: SGR マウスイベントスクロール | v0.1 完了 |
| #4 | Vim navigation keys | v0.1 完了 |
| #8 | Statusline: lualine-style mode indicator | v0.1 完了 |
| #5 | Visual mode and yank | v0.2 延期 |
| #6 | Multi-terminal support | v0.2 延期 |
| #7 | vte による自前 scrollback viewer | v0.2 延期 |

## 実装時の注意

- Kitty kbd protocol 対応: Esc=`\x1b[27u`、Ctrl+d=`\x1b[100;5u` 等も認識する
- smcup フィルタはチャンクをまたぐシーケンス未対応（実用上問題なし）
- PTY 出力後にステータスラインを再描画（SavePosition/RestorePosition）
- スクロールは 50ms スロットルで加速防止
- 詳細な設計ドキュメント: [docs/architecture.md](docs/architecture.md)
