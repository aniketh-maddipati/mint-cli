# mint-cli

Full-screen terminal UI for controlling AI agent runs from one place: **Claude** and **Codex** in PTYs, **Tinker** and **LM Studio** over OpenAI-compatible HTTP.

## Requirements

- Rust 1.85+ (edition 2024)
- `claude` and/or `codex` on `PATH` for PTY sessions
- LM Studio or Tinker API access for HTTP sessions

## Quick start

```bash
cargo run
```

Config is written on first run to the platform config dir (`~/.config/mint-cli/config.toml` on macOS/Linux).

## Sessions

Each session is a persistent workspace stored under `~/.local/share/mint-cli/sessions/`:

- **PTY sessions** — spawn `claude` or `codex` in a pseudo-terminal pane
- **HTTP sessions** — multi-turn chat against Tinker or LM Studio with streamed responses and run records

### Keys

| Key | Action |
|-----|--------|
| `Ctrl+Q` / `F10` | Quit |
| `F1` | Focus session list |
| `F2` | Focus output / prompt |
| `F3` | Focus controls |
| `F4` | Focus parameter scrubbers |
| `[` / `]` | Previous / next session |
| `n` | New session (cycles Tinker → LM Studio → Codex → Claude) |

Mouse clicks work on sessions, buttons, and scrubbers.

## Config

```toml
[claude]
command = "claude"

[codex]
command = "codex"

[http.tinker]
base_url = "https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1"
model = "tinker://YOUR_CHECKPOINT"
api_key_env = "TINKER_API_KEY"

[http.lmstudio]
base_url = "http://localhost:1234/v1"
model = "local-model"

[params]
temperature = 0.7
max_tokens = 2048.0
top_p = 1.0
```

Legacy `[lmstudio]` sections are migrated automatically.

## Development

```bash
cargo build
cargo test
```

Logs go to a file under the data directory (stdout is owned by the TUI).
