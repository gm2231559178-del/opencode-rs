# opencode-rs

Rust reimplementation of [OpenCode](https://github.com/anomalyco/opencode), an open-source AI coding agent.

## Status

v1 — core agent loop with streaming TUI:

| Component | Status |
|-----------|--------|
| CLI entrypoint (clap) | Done |
| Config system (JSONC) | Done |
| LLM providers (OpenAI, Anthropic, OpenRouter) | Done |
| Tool system (bash, read, write, edit, grep, glob, task) | Done |
| Session/agent loop (max 50 iterations) | Done |
| Streaming TUI (ratatui) | Done |
| Permission system (ask/allow/deny) | Done |
| Interrupt (Escape cancels) | Done |
| Undo (file snapshots + /undo) | Done |
| Session persistence (SQLite) | Done |
| Input history (Up/Down) | Done |
| Slash commands (/help, /new, /models, /sessions, /undo, /exit) | Done |
| Multi-line input | In progress |
| HTTP server | Not yet |
| LSP integration | Not yet |
| MCP integration | Not yet |
| Plugin system | Not yet |

## Quick Start

```bash
# Configure your API key
cat ~/.config/opencode/opencode.jsonc
{
  "model": "openrouter/nvidia/nemotron-3-ultra-550b-a55b:free",
  "provider": {
    "openrouter": { "api_key": "sk-or-v1-..." }
  }
}

# Run a prompt
cargo run -- "explain what this project does"

# Start the TUI
cargo run -- start
```

## Config

Config is loaded from `~/.config/opencode/opencode.jsonc` or `.opencode/opencode.jsonc`.

### Supported Providers

| Provider | Model prefix | Config key | API key env var |
|----------|-------------|------------|----------------|
| OpenAI | `openai/` | `openai` | `OPENAI_API_KEY` |
| Anthropic | `anthropic/` | `anthropic` | `ANTHROPIC_API_KEY` |
| OpenRouter | `openrouter/` | `openrouter` | `OPENAI_API_KEY` |

### Example: OpenRouter (OpenAI-compatible)

```jsonc
{
  "model": "openrouter/qwen/qwq-32b:free",
  "provider": {
    "openrouter": { "api_key": "sk-or-v1-..." }
  }
}
```

### Example: OpenAI

```jsonc
{
  "model": "openai/gpt-4o",
  "provider": {
    "openai": { "api_key": "sk-..." }
  }
}
```

### Example: Anthropic

```jsonc
{
  "model": "anthropic/claude-3-5-sonnet-latest",
  "provider": {
    "anthropic": { "api_key": "sk-ant-..." }
  }
}
```

## TUI Features

- **Streaming** — Tokens appear in real-time as the model generates them.
- **Status bar** — Bottom bar shows model name, prompt count, and idle/streaming state.
- **Input history** — Up/Down arrows recall previous prompts.
- **Tool details** — Tool calls (`tool>` yellow) and results (`result>` gray) shown inline.
- **Permissions** — Approve/deny `bash`, `write`, `edit` tool calls with `y`/`n`.
- **Interrupt** — Escape key cancels the current LLM request or tool execution.
- **Slash commands** — `/help`, `/new` (clear), `/models` (show model), `/sessions` (list saved), `/undo` (revert file change), `/exit` (quit).
- **Scroll** — PageUp/PageDown in the chat panel.

## CLI Commands

```
opencode-rs [OPTIONS] [PROMPT] [COMMAND]

Commands:
  start    Start the TUI
  run      Run a single prompt (non-interactive)
  config   Manage configuration
  version  Print version

Options:
  --provider <PROVIDER>  Override provider
  --model <MODEL>        Override model (e.g. "openai/gpt-4o")
  --log-level <LEVEL>    Log level [default: info]
```

## Project Structure

```
src/
├── main.rs           — Entry point, async main, dispatch
├── cli.rs            — CLI argument parsing (clap)
├── config.rs         — JSONC config loader
├── session.rs        — Agent conversation loop + undo + permission check
├── session_store.rs  — SQLite-backed session persistence
├── tui.rs            — Terminal UI (ratatui)
├── llm/
│   ├── mod.rs        — Provider factory
│   ├── provider.rs   — LLMProvider trait + StreamEvent types
│   ├── openai.rs     — OpenAI-compatible API client
│   └── anthropic.rs  — Anthropic Messages API client
└── tools/
    ├── mod.rs        — Tool trait + registry
    ├── bash.rs       — Shell command execution
    ├── read.rs       — File/directory reading
    ├── write.rs      — File writing
    ├── edit.rs       — String replacement editing
    ├── grep_tool.rs  — Content search (ripgrep)
    ├── glob_tool.rs  — File globbing (fd)
    └── task.rs       — Sub-agent delegation (stub)
```
