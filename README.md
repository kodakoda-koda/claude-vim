# cv — Vim-modal wrapper for Claude Code

A single-binary PTY wrapper that adds vim-style modal editing to [Claude Code](https://docs.anthropic.com/en/docs/claude-code). No Neovim or plugin dependencies.

```
Normal mode  →  Scroll conversation with j/k/Ctrl+d/u/gg/G
Insert mode  →  Text input (Enter = newline)
Esc          →  Switch to Normal mode
i            →  Switch to Insert mode
```

## Install

```bash
cargo install --git https://github.com/kodakoda-koda/claude-vim
```

Or build from source:

```bash
cargo build --release
./target/release/claude-vim
```

## Keybindings

### Normal mode

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll down / up |
| `Ctrl+d` / `Ctrl+u` | Half page down / up |
| `Ctrl+f` / `Ctrl+b` | Full page down / up |
| `gg` | Jump to top |
| `G` | Jump to bottom |
| `i` | Switch to Insert mode |
| `Enter` | Send message |
| `Ctrl+C` | Interrupt |

Scroll keys are consumed by cv. Other control keys and escape sequences pass through to Claude Code (e.g. `Shift+Tab` for permission mode).

### Insert mode

| Key | Action |
|-----|--------|
| `Esc` | Switch to Normal mode |
| `Enter` | Newline |
| Everything else | Raw passthrough to Claude Code |

## Statusline

A lualine-inspired statusline at the bottom of the terminal:

- **NORMAL** (blue) / **INSERT** (green) mode indicator
- Git branch name
- Version

## Requirements

- Rust 1.85+
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) (`claude` in PATH)
- Terminal with SGR mouse support (Kitty, iTerm2, WezTerm, Ghostty, Alacritty, etc.)

## Status

v0.1 — Core functionality works. Expect rough edges.

## License

MIT
