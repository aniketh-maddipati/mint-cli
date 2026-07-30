# mint-cli

Debuggable terminal workspace: real PTY sessions (Claude, Codex, shell commands), project-scoped timeline, honest fork/rerun — not HTTP chat, not tmux.

## Requirements

- Rust 1.85+ (edition 2024)
- `claude` and/or `codex` on `PATH` for agent panes
- Git repo with `origin` remote for project identity (falls back to path hash)

## Quick start

```bash
cargo run
```

Config is written on first run to `~/.config/mint-cli/config.toml`. Project data lives under `~/.local/share/mint-cli/projects/{git-remote-id}/`.

## Workspace

- **Default**: one full-screen Claude PTY, lazy auto-started on first render
- **Split**: `Ctrl+\` toggles Claude | Codex side-by-side
- **Command panes**: declare extra PTY panes in config (see below)
- **Timeline**: `F3` toggles project stage list at the bottom

### Keys

| Key | Action |
|-----|--------|
| `Ctrl+Q` / `F10` | Quit |
| `F3` | Toggle timeline |
| `Ctrl+\` | Toggle single / Claude\|Codex split |
| `[` / `]` | Previous / next pane |
| `Ctrl+R` | Restart active pane |

Keystrokes go directly to the focused PTY pane.

## Config

```toml
[claude]
command = "claude"

[codex]
command = "codex"

[[commands]]
label = "shell"
command = "bash"
args = []
```

Legacy chat-era config (`[http]`, `[params]`) is migrated automatically — HTTP and scrubber UI are removed.

## Development

```bash
cargo build
cargo test
```

Logs go to a file under the data directory (stdout is owned by the TUI).
