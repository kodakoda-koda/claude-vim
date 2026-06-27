# claude-vim (`cv`)

A minimal vim-modal wrapper for [Claude Code](https://docs.anthropic.com/en/docs/claude-code).

```
Insert mode  →  normal Claude Code operation (full passthrough)
Esc          →  Normal mode (scroll conversation with vim keys)
i            →  back to Insert mode
```

## Keybindings

| Key | Action |
|-----|--------|
| `Esc` | Enter Normal mode |
| `i` | Enter Insert mode |
| `j` / `k` | Scroll down / up |
| `Ctrl+d` / `Ctrl+u` | Scroll half page down / up |
| `Ctrl+f` / `Ctrl+b` | Scroll full page down / up |
| `g g` | Go to top |
| `G` | Go to bottom |
| `v` | Visual mode (select text) |
| `y` | Yank selection to clipboard |

## Requirements

- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed and in `$PATH`
- macOS (Linux support planned)

## Install

```bash
cargo install --git https://github.com/kodakoda-koda/claude-vim
```

Then run:

```bash
cv
```

## Status

Early development. Expect rough edges.

## License

MIT
