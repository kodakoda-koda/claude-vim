# CLAUDE.md

## プロジェクト概要

`claude-vim` は Claude Code に vim のモーダル操作を追加する PTY ラッパー。単一バイナリ、ターミナル非依存。

- Normal mode: スクロール（SGR マウスイベント経由）+ 数値プレフィックス（10j 等）
- Insert mode: テキスト入力（raw passthrough、Enter→改行変換）
- Cursor mode: hjkl でカーソル移動（テキスト選択の起点）
- Visual mode: 文字単位のテキスト選択 + yank（クリップボードコピー）

## ファイル構成

```
src/
├── main.rs        エントリポイント。raw mode、PTY 起動、SIGWINCH
├── app.rs         イベントループ。4モード管理、Esc 検出、Cursor/Visual ハイライト
├── pty.rs         portable-pty で claude 起動、smcup フィルタ、VirtualScreen feed
├── screen.rs      VirtualScreen（vte::Perform 実装）。PTY 出力をパースして grid 保持
├── scroll.rs      SGR マウスイベントでスクロール（Scroller）
├── input.rs       Normal/Cursor/Visual mode キーマッチング（raw bytes → Action）
└── statusline.rs  lualine 風ステータスライン（モード・ブランチ・バージョン）
```

## ビルド・実行

```bash
cargo build
cargo clippy
cargo test
./target/debug/claude-vim
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
| `j`/`k` | 1行スクロール（数値プレフィックス対応: `10j`） |
| `Ctrl+d`/`Ctrl+u` | 半ページ（rows/6 イベント） |
| `Ctrl+f`/`Ctrl+b` | 全ページ（rows/3 イベント） |
| `gg` / `G` | 最上部 / 最下部（Ctrl+Home/End） |
| `c` | Cursor mode へ |
| printable 文字 | 無視 |
| control/esc seq | PTY へ透過 |

### Cursor mode
| キー | 動作 |
|------|------|
| `h`/`j`/`k`/`l` | カーソル移動（1文字ハイライト） |
| `v` | Visual mode へ（カーソル位置から選択開始） |
| `Esc` | Normal mode へ戻る |

### Visual mode
| キー | 動作 |
|------|------|
| `h`/`j`/`k`/`l` | 選択範囲を文字単位で拡縮 |
| `y` | 選択テキストをクリップボードにコピー → Normal mode |
| `Esc` | 選択キャンセル → Normal mode |

## Issue 対応

| Issue | 内容 | 状態 |
|-------|------|------|
| #1 | PTY wrapper infrastructure | v0.1 完了 |
| #2 | Insert mode: raw passthrough | v0.1 完了 |
| #3 | Normal mode: SGR マウスイベントスクロール | v0.1 完了 |
| #4 | Vim navigation keys | v0.1 完了 |
| #8 | Statusline: lualine-style mode indicator | v0.1 完了 |
| #10 | Numeric prefix (10j, 5k) | v0.2 完了 |
| #7 | VirtualScreen (vte) | v0.2 完了 |
| #5 | Visual mode and yank | v0.2 完了 |
| #6 | Multi-terminal support | クローズ（v0.1 でターミナル非依存化） |
| #11 | Vim cursor motion ($, ^, w, b, e) | v0.3 予定 |

## 実装時の注意

- Kitty kbd protocol 対応: Esc=`\x1b[27u`、Ctrl+d=`\x1b[100;5u` 等も認識する
- smcup フィルタはチャンクをまたぐシーケンス未対応（実用上問題なし）
- PTY 出力後にステータスラインを再描画（SavePosition/RestorePosition）
- スクロールは 50ms スロットルで加速防止
- Cursor/Visual mode のハイライト消去は VirtualScreen からオリジナル内容を読んで上書き復元
- VirtualScreen は Arc<Mutex> で PTY reader スレッドと共有
- 詳細な設計ドキュメント: [docs/architecture.md](docs/architecture.md)
