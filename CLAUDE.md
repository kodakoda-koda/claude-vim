# CLAUDE.md

## プロジェクト概要

`cv` は Claude Code に vim のモーダル操作を追加するスタンドアロンの PTY ラッパー。Neovim やプラグインには依存しない単一バイナリ。

```
Insert mode  →  Claude Code への完全 raw byte パススルー
Esc          →  Normal mode（Kitty の scroll API で会話履歴を vim キーでスクロール）
i            →  Insert mode に戻る
```

## ロードマップ

| バージョン | スコープ |
|---|---|
| **v0.1** | Kitty 限定。Insert = raw passthrough、Normal = Kitty scroll |
| **v0.2** | vte による自前 scrollback viewer、Visual mode + yank、他ターミナル対応 |

---

## Claude Code の挙動（実測・確定事項）

### 起動時に送るエスケープシーケンス（順番通り）

```
\x1b7              DEC カーソル保存
\x1b[r             スクロール領域リセット
\x1b8              DEC カーソル復元
\x1b[?25h          カーソル表示
\x1b[?1049h        ★ alternate screen 入場（SMCUP）← cv でフィルタ必要
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
\x1b[?2026h        ★ synchronized output 開始（フレーム境界マーカー）
```

### Kitty keyboard protocol の影響

Claude Code が `\x1b[>1u` を PTY 出力として送ると、cv がそれをそのまま実際の Kitty 端末に転送する。Kitty は「拡張キーシーケンスを使え」という指示を受け取り、以後 Shift+Enter などを `\x1b[13;2u` のような拡張シーケンスで送るようになる。

**このため Insert mode の実装は crossterm イベント → バイト再エンコードではなく、raw byte passthrough が必須。** crossterm は Kitty keyboard protocol の拡張シーケンスを再現できない。

### Claude Code の全インタラクティブ状態

| 状態 | キー操作 | cv としての対応 |
|------|---------|----------------|
| 通常会話（入力中） | Enter=送信, Ctrl-C=中断 | Insert: raw passthrough |
| /resume ピッカー | j/k=選択, Enter=決定, Esc=キャンセル | Insert: raw passthrough |
| /model ピッカー | 同上 | Insert: raw passthrough |
| /config トグル | Enter/Space=切替, Esc=閉じる | Insert: raw passthrough |
| /permissions | Tab=移動, Enter=操作 | Insert: raw passthrough |
| /agents, /workflows | ↑↓=選択, f=フィルタ, x=削除 | Insert: raw passthrough |
| パーミッション確認 | y/n or Enter/Esc | Insert: raw passthrough |
| Edit プレビュー | Enter=確認, Esc=拒否 | Insert: raw passthrough |
| Claude Code 内蔵 Normal mode | j/k/Ctrl+d/u/f/b/gg/G=スクロール | Insert: raw passthrough |
| Claude 応答ストリーミング中 | Ctrl-C=中断 | Insert: raw passthrough |
| **cv Normal mode** | j/k→Kitty scroll | Normal: kitty scroll のみ |

**結論：Insert mode = raw passthrough とするだけで、すべての Claude Code 内部状態が自動的に正しく動く。**

---

## 確定した設計方針

### Insert mode は raw byte passthrough

crossterm のイベントモデルを使わず、stdin の生バイトをそのまま PTY に流す。

理由：
- Kitty keyboard protocol（`\x1b[13;2u` など）を正確に転送できる
- マウスイベントが自動的に通る（`\x1b[?1002h` など Claude Code が有効化）
- bracketed paste、フォーカストラッキングも透過
- /resume の j/k 選択、パーミッション確認、すべてのダイアログが正しく動く

**Esc の扱い（設計決定 A 案を採用）：**
- Esc が単独で来た場合（後続バイトなし、50ms タイムアウト）→ cv を Normal mode に切替。Claude Code には Esc を **送らない**
- これにより、ダイアログを誤キャンセルしない
- Claude Code 内蔵 Normal mode（Claude Code 自身の j/k スクロール）は cv Normal mode の間は一時停止状態になるが、i で cv Insert に戻れば再開

### Normal mode のスクロールは Kitty remote control を使う（v0.1）

```bash
# 正しい構文（"up N lines" ではなく数値+単位+方向）
kitty @ --to $KITTY_LISTEN_ON scroll-window 1l-   # k: 1行上
kitty @ --to $KITTY_LISTEN_ON scroll-window 1l    # j: 1行下
kitty @ --to $KITTY_LISTEN_ON scroll-window 1p-   # Ctrl+u: 半ページ上（1p=1ページ）
kitty @ --to $KITTY_LISTEN_ON scroll-window 1p    # Ctrl+d: 半ページ下
kitty @ --to $KITTY_LISTEN_ON scroll-window start # gg: 最上部
kitty @ --to $KITTY_LISTEN_ON scroll-window end   # G: 最下部
```

**単位：** `l`=行（デフォルト）、`p`=ページ、`u`=アンスクロール、`r`=プロンプトへ
**方向：** `-` サフィックスで逆方向（上方向）

socket は `$KITTY_LISTEN_ON` 環境変数から取得。未設定の場合は `unix:/tmp/mykitty` にフォールバック。

### smcup フィルタ

Claude Code が送る `\x1b[?1049h`（alternate screen 入場）を cv でフィルタして通常スクリーンに描画させる。これにより内容が Kitty のスクロールバックに蓄積し、`scroll-window` で閲覧可能になる。

フィルタ対象：
- `\x1b[?1049h` / `\x1b[?1049l`（smcup/rmcup）
- `\x1b[?1047h` / `\x1b[?1047l`（旧形式）
- `\x1b[?47h` / `\x1b[?47l`（さらに旧形式）
- `\x1b[3J`（スクロールバック消去）

---

## scrollback 手法の比較・評価（検証済み）

| 手法 | 実現可能？ | 品質 | 端末依存 | 備考 |
|------|-----------|------|---------|------|
| **①smcup filter + kitty scroll-window** | ✅ | △ TUI chrome 混入 | Kitty のみ | v0.1 採用 |
| **②kitty @ get-text** | ✅ | ◯ Kitty が描画後のテキスト | Kitty のみ | v0.1.5 候補。smcup filter と併用 |
| **③vte 自前エミュレーション** | ✅ PoC PASS | ◎ 完全クリーン | 不要 | **v0.2 採用予定** |
| ④sync output フレーム差分 | △ | ◯ | 不要 | ③と本質的に同じ（vte 必要）。独立手法として不採用 |
| ⑤tmux バックエンド | ❌ tmux not found | ◎ | tmux 必須 | 環境依存が強く不採用 |

### 手法③ vte PoC 結果

```
入力: \x1b[2J\x1b[H + "line 1\r\n" ～ "line 6 (current)"（5行スクリーン）
結果: scrollback = ["line 1"]、screen row0 = "line 2" ... row4 = "line 6 (current)"
→ PASS: 自然にスクロールアウトした行を正確にキャプチャ
```

実装概要（`src/screen.rs` として v0.2 で追加）：
- `vte::Parser` + `vte::Perform` トレイトを実装した `VirtualScreen` 構造体
- 文字グリッド（`Vec<Vec<Cell>>`）とスクロール領域を管理
- LF/SU/IL/DL 等でスクロールが発生した行を `VecDeque<String>` に取り込む
- smcup (`\x1b[?1049h`) を検知して alternate screen 用のサブグリッドに切り替え
- v0.2 の Normal mode viewer と Visual mode/yank で使用

---

## アーキテクチャ

```
┌─────────────────────────────────────────────────┐
│  cv (wrapper process)                           │
│                                                 │
│  Insert mode:                                   │
│    stdin (raw bytes) ──→ PTY(claude)            │
│    ※Esc 単独（50ms timeout）のみ intercepto    │
│    PTY(claude) output ──→ filter ──→ stdout     │
│                                                 │
│  Normal mode:                                   │
│    j/k/etc ──→ kitty @ scroll-window            │
│    i/Ctrl-C/Enter ──→ PTY or mode 切替          │
│    その他のキー ──→ 無視                        │
└──────────────┬──────────────────────────────────┘
               │ PTY
┌──────────────▼──────────────────────────────────┐
│  claude (subprocess)                            │
└─────────────────────────────────────────────────┘
```

**stdin reader スレッド:**
- 別スレッドで stdin を生バイト読み取り → チャンネルで main loop へ
- Insert mode: バイトをそのまま PTY へ。Esc 単独検出（後続バイトなし 50ms）で Normal mode 切替
- Normal mode: バイトを手動でキーマッチング（j/k/g/G/i/Enter/Ctrl+C）

**SIGWINCH:**
- `signal-hook` crate で AtomicBool フラグ → メインループでチェックして PTY リサイズ

---

## ファイル構成

```
src/
├── main.rs     エントリポイント。ターミナルサイズ取得、raw mode 設定
├── app.rs      メインイベントループ。モード管理、イベント振り分け
├── pty.rs      portable-pty で claude をサブプロセス起動、reader スレッド
├── kitty.rs    kitty @ scroll-window ラッパー（正しい構文で）
└── input.rs    Normal mode キーマッチング（raw bytes → アクション）
```

v0.2 追加予定：
```
src/
└── screen.rs   VirtualScreen（vte Perform 実装）、scrollback バッファ
```

---

## 主要な依存 crate

- `portable-pty` — PTY 管理（WezTerm 製）
- `crossterm` — raw mode 制御、ターミナルサイズ取得（イベント読み取りには使わない）
- `anyhow` — エラー処理
- `signal-hook` — SIGWINCH ハンドリング
- `vte` — v0.2 で VirtualScreen 実装に使用（Cargo.toml に既に記載）

---

## キーバインド（確定）

### cv Insert mode
| キー | 動作 |
|------|------|
| `Esc`（単独・50ms timeout） | cv Normal mode へ切替（Claude Code には送らない） |
| その他すべて | raw bytes をそのまま PTY へ |

### cv Normal mode
| キー | 動作 |
|------|------|
| `i` | cv Insert mode へ切替 |
| `j` | 1行下スクロール |
| `k` | 1行上スクロール |
| `Ctrl+d` | 半ページ下 |
| `Ctrl+u` | 半ページ上 |
| `Ctrl+f` | 全ページ下 |
| `Ctrl+b` | 全ページ上 |
| `g g` | 最上部へ |
| `G` | 最下部へ |
| `Enter` | `\r` を PTY へ送信（メッセージ送信） |
| `Ctrl+C` | `\x03` を PTY へ送信（中断） |

---

## Issues との対応

| Issue | 内容 | バージョン | 状態 |
|-------|------|-----------|------|
| #1 | PTY wrapper infrastructure | v0.1 | 未着手 |
| #2 | Insert mode: raw passthrough | v0.1 | 未着手 |
| #3 | Normal mode: Kitty scroll API | v0.1 | 未着手 |
| #4 | Vim navigation keys | v0.1 | 未着手 |
| #5 | Visual mode and yank | v0.2 | 延期 |
| #6 | Multi-terminal support | v0.2 | 延期 |
| #7 | vte による自前 scrollback viewer | v0.2 | 未作成 |

---

## 開発ワークフロー

- `feat/v0.1` ブランチで Issue #1〜#4 を一括実装 → master へ PR
- `cargo build` でビルド確認、`cargo clippy` でリント

## ビルド

```bash
cargo build           # debug
cargo build --release
./target/debug/cv     # 実行
```

## 実装時の注意

- `$KITTY_LISTEN_ON` が設定されていることを起動時にチェックし、未設定なら `unix:/tmp/mykitty` にフォールバック
- smcup フィルタはチャンクをまたぐシーケンスへの対処が不完全（実用上は問題なし）
- Normal mode 中も Claude Code の PTY 出力は流れ続ける。メインループで drain して stdout に書き続ける
- Kitty の設定: `allow_remote_control yes` + `listen_on unix:/tmp/mykitty` が必要（設定済み）
