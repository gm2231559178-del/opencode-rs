# GAP — opencode-rs vs opencode

Features from the original [opencode](https://github.com/anomalyco/opencode) project (`/workspaces/opencode`) that are not yet ported to opencode-rs.

---

## P1 — TUI usability

| Gap | Status in opencode-rs | Location in opencode |
|-----|-----------------------|----------------------|
| Multi-line input | `[x]` Shift+Enter newline, Enter submits | `packages/tui/` |
| Copy last response | `[x]` Ctrl+Y via `arboard` | `packages/tui/src/clipboard.ts` |

## P2 — Feature parity

| Gap | Status in opencode-rs | Location in opencode |
|-----|-----------------------|----------------------|
| Session management (continue, fork, rename, delete) | `[x]` Commands: `/session load`, `/session fork`, `/session delete` | `packages/core/src/session/store.ts` |
| Plan mode (read-only agent preset) | `[x]` `/plan` toggle, denies write/edit/bash | `packages/opencode/src/session/prompt/plan-mode.txt` |
| Model/agent picker | `[x]` `/model`, `/agent` commands | `packages/tui/src/component/dialog-model.tsx` |
| Context compaction | `[x]` `/compact` drops old tool result messages | `packages/core/src/config/compaction.ts` |
| Diff view | `[x]` `/diff` shows line-diff of last edit | `packages/tui/src/feature-plugins/system/diff-viewer*.tsx` |
| File autocomplete (`@` mentions) | `[x]` `@` triggers `fd` file search, Tab/Enter to select | `packages/tui/src/component/prompt/autocomplete.tsx` |
| Subagents | `[x]` `task` tool spawns real sub-agents with LLM calls | `packages/core/src/agent.ts` |

## P3 — Infrastructure

| Gap | Status in opencode-rs | Location in opencode |
|-----|-----------------------|----------------------|
| HTTP server (`opencode serve`) | `[x]` axum-based, `/health`, `/chat`, `/sessions` | `packages/cli/src/commands/handlers/serve.ts` |
| ACP protocol (stdin/stdout JSON-RPC) | `[x]` `opencode acp`, methods: `chat`, `sessions/list`, `sessions/get`, `ping` | `packages/opencode/src/acp/` |
| Config merging (layered) | `[x]` Global → project → env → CLI | `packages/core/src/config.ts` |
| Environment variables (`OPENCODE_*`) | `[x]` `OPENCODE_MODEL`, `OPENCODE_PROVIDER_API_KEY`, `OPENCODE_SHELL`, etc. | `packages/core/src/flag/flag.ts` |
| Session sharing | `[x]` `/share`, `/share import`, `/share list` | `packages/core/src/share/sql.ts` |
| Stats tracking | `[x]` `/stats` shows token/prompt/call counts | `packages/stats/` |
| mDNS discovery | `[x]` `_opencode._tcp.local` via `mdns-sd` | `packages/opencode/src/server/mdns.ts` |
| MCP support | `[ ]` | `packages/core/src/config/mcp.ts` |
| Plugin system | `[ ]` | `packages/core/src/plugin.ts` |
| LSP integration | `[ ]` | `packages/core/src/config/lsp.ts` |
| Theme system | `[x]` 6 themes, `/theme` command | `packages/tui/src/theme/` |
| Notifications | `[x]` Terminal bell on done/error, `/notify` toggle | `packages/tui/src/attention.ts` |

---

Legend: `[ ]` not started | `[~]` in progress | `[x]` done
