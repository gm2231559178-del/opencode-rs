# UI/UX Gap Analysis — opencode-rs vs opencode

## Legend
- `[ ]` not started | `[~]` in progress | `[x]` done

---

## P0 — Critical (agent loop)

- `[x]` **Streaming** — Tokens rendered in real-time via channel-based event loop. `LLMProvider::stream()` → `mpsc::channel` → TUI draw loop.
- `[x]` **Permission system** — `ask`/`allow`/`deny` per tool. TUI shows approval dialog (y/n) for `bash`/`edit`/`write` with `PermissionAction` channel.
- `[x]` **Interrupt** — Escape key sets `AtomicBool` checked between tool calls and LLM requests.
- `[x]` **Undo** — File snapshots before `edit`/`write` (`read_to_string`). `/undo` restores via `undo_last()`. Redo not yet implemented.

## P1 — High (TUI usability)

- `[x]` **Status bar** — Bottom bar with model badge, agent badge (secondary), streaming indicator, plan/leader mode tags, theme:count, proper background_element coloring.
- `[x]` **Tool execution details** — Tool calls shown as `tool>` (yellow dim) and results as `result>` (dark gray dim) entries with arg previews.
- `[x]` **Card-style messages** — Messages rendered with left border markers (▎) and role-colored backgrounds for visual hierarchy.
- `[x]` **Input history** — Up/Down navigates previous prompts (`Vec<String>` with index tracking).
- `[x]` **Theme colors** — Expanded to 20 tokens matching original visual language (background_panel, background_element, border_active, text_muted, secondary for agent tags).
- `[x]` **Slash commands** — `/help`, `/new`, `/models`, `/sessions`, `/undo`, `/exit`, `/plan`, `/compact`, `/theme`, `/diff`, `/agent`, `/share`, `/share import`, `/share list`, `/stats`, `/mcp`, `/plugin`, `/diagnostics`, `/notify`, `/session load`, `/session fork`, `/session rename`, `/session delete` all implemented.
- `[x]` **Multi-line input** — Enter submits, Shift+Enter inserts newline, Esc clears.
- `[x]` **Copy last response** — Ctrl+Y / leader+y copies last assistant message to clipboard.

## P2 — Medium (feature parity)

- `[x]` **Sidebar** — 36-col toggleable left panel with 5 collapsible sections (Context, MCP, LSP, Todo, Files) using background_panel.
- `[x]` **Command palette** — Ctrl+P/leader+k opens filterable popup with 20 categorized commands.
- `[x]` **Text prompt dialog** — Inline text input dialog for rename/tag operations.
- `[x]` **Session management** — Rename, delete via command palette with confirm dialogs.
- `[x]` **Session persistence** — SQLite-backed store (`~/.config/opencode-rs/sessions.db`). Auto-saves on Done/Error. `/sessions` lists recent sessions.
- `[x]` **Session management** — Continue (`/session load`), fork (`/session fork`), rename (`/session rename`), delete (`/session delete`) existing sessions.
- `[x]` **Plan mode** — Read-only agent preset: `edit=deny`, `bash=ask`, `write=deny`, `apply_patch=deny`. Toggle from input (/plan).
- `[x]` **Diff view** — Inline display of additions/removals for file edits (`/diff`).
- `[x]` **Model/agent/theme picker dialogs** — Leader m/a/t opens selection dialogs with search, category grouping, footer hints.
- `[x]` **Session list dialog** — Leader s opens saved sessions list with search.
- `[x]` **MCP status dialog** — Leader c shows MCP tools connected with searchable list.
- `[x]` **Stash/prompt dialog** — Leader p shows quick-access stashed commands.
- `[x]` **Status overview dialog** — Leader ? shows session status (model, theme, plan mode, notifications, stats).
- `[x]` **Help dialog** — Leader h shows keybindings summary.
- `[x]` **Context compaction** — Auto-triggered when context tokens exceed 50k (streaming complete). Manual via `/compact`.
- `[x]` **File autocomplete** — `@` triggers fuzzy file search via `fd` + reference names from config. `#L` line suffix preserved on selection.
- `[ ]` **Subagents** — `@general`, `@explore`, `@scout` mention from input to delegate tasks.

## P3 — Low (infrastructure)

- `[x]` **HTTP server** — `opencode serve` with axum-based REST API + endpoints `/health`, `/chat`, `/sessions`, `/sessions/:id`.
- `[x]` **ACP protocol** — Line-delimited JSON-RPC over stdin/stdout. Methods: `chat`, `sessions/list`, `sessions/get`, `ping`.
- `[x]` **Config merging** — Layered config: global → project → env var → CLI flag.
- `[x]` **Environment variables** — `OPENCODE_*` support (model, api key, base url, permissions, etc.).
- `[x]` **Session sharing** — Share sessions via `/share` (SQLite-backed `shared_sessions` table).
- `[x]` **Stats tracking** — Token usage, cost, tool frequency (`/stats`, `UsageStats`).
- `[x]` **mDNS discovery** — Zero-config local network server discovery via `mdns-sd`.
- `[x]` **MCP support** — Model Context Protocol server connections via config-driven JSON-RPC.
- `[x]` **Plugin system** — Custom tools and commands via config-driven process plugins.
- `[x]` **LSP integration** — `/diagnostics <file>` with per-extension LSP server launch.
- `[x]` **Theme system** — Configurable colors (tokyonight, catppuccin, gruvbox, etc.).
- `[x]` **Notifications** — Desktop alerts via notify-rust when response completes or error occurs (toggled via `/notify`). Falls back to terminal bell.

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
Before any `edit` or `write` tool call, save the original file content for undo. On `/undo`, restore the saved content.
