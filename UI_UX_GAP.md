# UI/UX Gap Analysis — opencode-rs vs opencode

## Legend
- `[ ]` not started | `[~]` in progress | `[x]` done

---

## P0 — Critical (agent loop)

- `[x]` **Streaming** — Tokens rendered in real-time via channel-based event loop. `LLMProvider::stream()` → `mpsc::channel` → TUI draw loop.
- `[x]` **Permission system** — `ask`/`allow`/`deny` per tool. TUI shows approval dialog (y/n) for `bash`/`edit`/`write` with `PermissionAction` channel.
- `[x]` **Interrupt** — Escape key sets `AtomicBool` checked between tool calls and LLM requests.
- `[x] **Undo** — File snapshots before `edit`/`write` (`read_to_string`). `/undo` restores via `undo_last()`. Redo not yet implemented.

## P1 — High (TUI usability)

- `[x]` **Status bar** — Bottom bar showing model name, prompt count, and idle/streaming state.
- `[x]` **Tool execution details** — Tool calls shown as `tool>` (yellow dim) and results as `result>` (dark gray dim) entries with arg previews.
- `[x]` **Input history** — Up/Down navigates previous prompts (`Vec<String>` with index tracking).
- `[x]` **Slash commands** — `/help`, `/new`, `/models`, `/sessions`, `/undo`, `/exit` all implemented.
- `[~]` **Multi-line input** — Enter submits, Esc clears. Shift+Enter newline not yet implemented.
- `[ ]` **Copy last response** — Ctrl+Y or leader+y copies last assistant message to clipboard.

## P2 — Medium (feature parity)

- `[x]` **Session persistence** — SQLite-backed store (`~/.config/opencode-rs/sessions.db`). Auto-saves on Done/Error. `/sessions` lists recent sessions.
- `[ ]` **Session management** — Continue, fork, rename, delete existing sessions.
- `[ ]` **Plan mode** — Read-only agent preset: `edit=deny`, `bash=ask`. Toggle from input (Tab).
- `[ ]` **Model/agent picker** — Dialog to switch model or agent mid-session (leader+m / leader+a).
- `[ ]` **Context compaction** — Auto-trigger when approaching token limit. Manual via `/compact`.
- `[ ]` **Diff view** — Inline display of additions/removals for file edits.
- `[ ]` **File autocomplete** — `@` triggers fuzzy file search within the project.
- `[ ]` **Subagents** — `@general`, `@explore`, `@scout` mention from input to delegate tasks.

## P3 — Low (infrastructure)

- `[ ]` **HTTP server** — `opencode serve` with REST API + SSE event stream.
- `[ ]` **ACP protocol** — Line-delimited JSON over stdin/stdout for IDE integration.
- `[ ]` **Config merging** — Layered config: global → project → env var → CLI flag.
- `[ ]` **Environment variables** — `OPENCODE_*` support (model, config path, permissions, etc.).
- `[ ]` **Session sharing** — Share sessions via URL (opncd.ai/s/<id>).
- `[ ]` **Stats tracking** — Token usage, cost, tool frequency (`opencode stats`).
- `[ ]` **mDNS discovery** — Zero-config local network server discovery.
- `[ ]` **MCP support** — Model Context Protocol servers (local + remote + OAuth).
- `[ ]` **Plugin system** — Custom tools and commands via npm modules.
- `[ ]` **LSP integration** — goToDefinition, findReferences, hover, etc.
- `[ ]` **Theme system** — Configurable colors (tokyonight, catppuccin, gruvbox, etc.).
- `[ ]` **Notifications** — Desktop alerts when terminal is blurred (attention system).

---

## Architecture Notes

### Streaming
The `LLMProvider` trait already has `stream()` returning `BoxStream<'static, LLMEvent>`. The TUI needs to:
1. Spawn the prompt as a background task
2. Receive events via a channel (mpsc)
3. Update the current assistant message incrementally in the draw loop
4. Handle `text` events (append to buffer), `tool_use` events (show tool call), `tool_result` events (append result)

### Cancellation
Add an `AtomicBool` (`cancelled`) shared between the agent loop and the TUI event handler. On Escape keypress, set `cancelled = true`. The agent loop checks this flag before each tool call and before the next LLM request.

### Permission System
Add a `permission` field to each tool's `execute()`. The session prompts the user via a dialog when the tool's action is `ask`. The dialog offers: allow-once, allow-always, reject.

### Snapshots
Before any `edit` or `write` tool call, run `git add -A && git stash` (or similar) to snapshot the current state. On undo, reapply the snapshot.
