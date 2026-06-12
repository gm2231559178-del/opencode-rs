# GAP — opencode-rs vs opencode

Features from the original [opencode](https://github.com/anomalyco/opencode) project (`/workspaces/opencode`) that are not yet ported to opencode-rs.

---

## P1 — TUI usability

| Gap | Status in opencode | Location in opencode |
|-----|--------------------|----------------------|
| Multi-line input | `[~]` Shift+Enter inserts newline, Enter submits | `packages/tui/` |
| Copy last response | `[ ]` `<leader>y` via OSC52 / clipboardy / xclip | `packages/tui/src/clipboard.ts` |

## P2 — Feature parity

| Gap | Status in opencode | Location in opencode |
|-----|--------------------|----------------------|
| Session management (continue, fork, rename, delete) | `[ ]` SQLite-backed, dialog for list/search/pin | `packages/core/src/session/store.ts`<br>`packages/tui/src/component/dialog-session-list.tsx`<br>`packages/tui/src/component/dialog-session-rename.tsx` |
| Plan mode (read-only agent preset) | `[ ]` System prompt template, `plan_exit` tool | `packages/opencode/src/session/prompt/plan-mode.txt` |
| Model/agent picker | `[ ]` `<leader>m` / `<leader>a` dialogs with fuzzy search | `packages/tui/src/component/dialog-model.tsx`<br>`packages/tui/src/component/dialog-agent.tsx` |
| Context compaction | `[ ]` Configurable auto/manual via `<leader>c` | `packages/core/src/config/compaction.ts` |
| Diff view | `[ ]` Full TUI diff viewer with file tree, hunk nav, syntax highlighting | `packages/tui/src/feature-plugins/system/diff-viewer*.tsx` |
| File autocomplete (`@` mentions) | `[ ]` Fuzzy file search with `fuzzysort` + frecency | `packages/tui/src/component/prompt/autocomplete.tsx` |
| Subagents (`@general`, `@explore`, `@scout`) | `[ ]` Agents with `mode: "subagent"`, stream in separate tabs | `packages/core/src/agent.ts` |

## P3 — Infrastructure

| Gap | Status in opencode | Location in opencode |
|-----|--------------------|----------------------|
| HTTP server (`opencode serve`) | `[ ]` REST API + OpenAPI spec + event streaming | `packages/cli/src/commands/handlers/serve.ts`<br>`packages/server/src/`<br>`packages/opencode/src/server/server.ts` |
| ACP protocol (stdin/stdout JSON-RPC) | `[ ]` Line-delimited JSON-RPC for IDE integration | `packages/opencode/src/acp/` |
| Config merging (layered) | `[ ]` Global → project → env → CLI flags | `packages/core/src/config.ts` |
| Environment variables (`OPENCODE_*`) | `[ ]` Config path, permissions, server, experimental flags | `packages/core/src/flag/flag.ts` |
| Session sharing | `[ ]` SQLite-backed share table, configurable (manual/auto/disabled) | `packages/core/src/share/sql.ts` |
| Stats tracking | `[ ]` Dedicated stats app with DB, API, web UI | `packages/stats/` |
| mDNS discovery | `[ ]` `opencode.local` service publication | `packages/opencode/src/server/mdns.ts` |
| MCP support | `[ ]` Local + remote + OAuth2 servers, TUI dialog | `packages/core/src/config/mcp.ts`<br>`packages/tui/src/component/dialog-mcp.tsx` |
| Plugin system | `[ ]` npm module hooks (agent, command, skill, provider, env) | `packages/core/src/plugin.ts`<br>`packages/core/src/plugin/boot.ts` |
| LSP integration | `[ ]` Per-server config, watcher integration | `packages/core/src/config/lsp.ts` |
| Theme system | `[ ]` 33 built-in themes, live preview, TUI selection dialog | `packages/tui/src/theme/`<br>`packages/tui/src/component/dialog-theme-list.tsx` |
| Notifications | `[ ]` Audio (6 sound packs) + desktop notifications on blur | `packages/tui/src/attention.ts`<br>`packages/tui/src/feature-plugins/system/notifications.ts` |

---

Legend: `[ ]` not started | `[~]` in progress | `[x]` done
