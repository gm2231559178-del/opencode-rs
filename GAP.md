# GAP — opencode-rs vs opencode

Real feature gaps between the Rust reimplementation and the original [opencode](https://github.com/anomalyco/opencode).

---

## P0 — Agent Loop

| Gap | Status | Original location | Notes |
|-----|--------|-------------------|-------|
| Session runner (orchestration) | `[~]` | `packages/core/src/session/runner/` | Rust has basic loop but no run-coordinator, context-epoch, compaction |
| Event system | `[ ]` | `packages/core/src/event/` | Typed events, aggregation, versioning, sync |
| Session compaction | `[~]` | `packages/core/src/session/compaction.ts` | Rust has `/compact` but no auto-trigger or summarization |
| PTY/Terminal emulation | `[ ]` | `packages/core/src/pty/` | Full PTY with terminal emulation, bun/node backends |
| Tool output store | `[ ]` | `packages/core/src/tool-output-store.ts` | Bounded truncation for large tool outputs |

## P1 — Provider Ecosystem

| Gap | Status | Original location | Notes |
|-----|--------|-------------------|-------|
| 30+ providers | `[-]` | `packages/core/src/plugin/provider/*.ts` | Rust only has OpenAI + Anthropic — intentionally out of scope for rewrite |
| GitHub Copilot provider | `[-]` | `packages/core/src/github-copilot/` | Intentional gap |
| Provider catalog | `[-]` | `packages/core/src/catalog.ts` | Intentional gap |
| Model request system | `[-]` | `packages/core/src/model-request.ts` | Intentional gap |

## P1 — TUI Richness

| Gap | Status | Original location | Notes |
|-----|--------|-------------------|-------|
| Sidebar plugins (files, MCP, LSP, todos, context) | `[ ]` | `packages/tui/src/feature-plugins/sidebar/` | 5 sidebar panels with live status |
| Command palette | `[~]` | `packages/tui/src/component/command-palette.tsx` | `/` slash-command autocomplete (Tab/Enter to select) |
| Leader key system | `[x]` | `packages/tui/src/keymap.tsx` | Space leader key shows menu; f/s/m/a/t/h/p/c/%/n/d/e/q/? actions |
| Dialogs (agent, model, theme, session, MCP, stash, skill, status, help) | `[~]` | `packages/tui/src/component/dialog-*.tsx` | Agent, model, theme, session list, MCP status, stash, skill, status overview, help, alert, confirm dialogs |
| Autocomplete (`@` file, `#L` lines, frecency, references) | `[~]` | `packages/tui/src/component/prompt/autocomplete.tsx` | `@` triggers fd + directory indicator + reference candidates |
| Thinking/reasoning display | `[x]` | `packages/tui/src/context/thinking.ts` | Collapsible reasoning blocks, Ctrl+R toggle |
| Diff viewer (side-by-side/stacked) | `[x]` | `packages/tui/src/feature-plugins/system/diff-viewer.tsx` | Scrollable overlay with +/- colors, arrow/PgUp/PgDn/Home/End |
| Audio/sound system | `[ ]` | `packages/tui/src/audio.ts` | Events: question, permission, error, done |
| Clipboard integration | `[x]` | `packages/tui/src/context/clipboard.tsx` | Yank last response (Ctrl+Y) |
| Editor integration | `[x]` | `packages/tui/src/editor.ts`, `editor-zed.ts` | Ctrl+E opens last edited file in $EDITOR/$VISUAL/vi |
| Syntax highlighting | `[x]` | `packages/tui/src/context/theme.ts` | Code blocks in assistant messages parsed and colorized |
| Toast notifications | `[x]` | `packages/tui/src/ui/toastx.ts` | Inline toasts for actions (Ctrl+Y, Ctrl+R, Ctrl+O) |
| Scroll acceleration | `[x]` | `packages/tui/src/util/scroll.ts` | PageUp/PageDown |
| Multi-line input | `[x]` | `packages/tui/` | Shift+Enter newline, Ctrl+Enter submit |
| Tool output collapse | `[x]` | `packages/tui/src/util/collapse-tool-output.ts` | Ctrl+O collapse/expand |
| Fade-in animations | `[~]` | `packages/tui/src/util/signal.ts` | Age-based dim ramp (0→10 frames) for new assistant/reasoning messages |

## P2 — Infrastructure

| Gap | Status | Original location | Notes |
|-----|--------|-------------------|-------|
| Database layer (Drizzle ORM) | `[ ]` | `packages/core/src/database/` | Schema management, migrations |
| Daemon/service management | `[ ]` | `packages/cli/src/services/daemon.ts` | Background server, password auth, health checks |
| HTTP server API completeness | `[~]` | `packages/cli/src/commands/handlers/serve.ts` | Rust has 3 routes, original has 16 API groups |
| WebSocket transport | `[ ]` | `packages/cli/src/services/daemon.ts` | Real-time streaming via WS upgrade |
| File watcher | `[x]` | `packages/core/src/filesystem/watcher.ts` | `notify`-based recursive watcher, toast on change |
| LSP integration depth | `[~]` | `packages/core/src/lsp/` | Rust has basic `/diagnostics` only; missing goToDef, hover, references |
| MCP OAuth flow | `[ ]` | `packages/core/src/config/mcp.ts` | client_id/secret/scope/callback_port for remote MCP |
| Plugin system depth | `[~]` | `packages/core/src/plugin/` | Rust has simple process plugins; missing boot, env, provider, skill, TUI plugins |
| TUI plugin runtime | `[ ]` | `packages/tui/src/plugin/` | Plugin slots, API, adapters, command shim |

## P2 — Session/Workspace

| Gap | Status | Original location | Notes |
|-----|--------|-------------------|-------|
| Workspace system | `[ ]` | `packages/core/src/workspace.ts` | Multi-workspace with ID prefix `wrk_` |
| Control plane | `[ ]` | `packages/core/src/control-plane/` | Workspace CRUD, session movement |
| Account/auth system | `[ ]` | `packages/core/src/account/` | Device code flow, auth tokens |
| Credential system (encrypted) | `[ ]` | `packages/core/src/credential/` | Encrypted API key storage |
| Reference system | `[~]` | `packages/core/src/config/reference.ts` | External dir/git repo references via config with `@name` autocomplete |
| Instruction context (AGENTS.md) | `[x]` | `packages/core/src/instruction-context.ts` | Auto-discovered from cwd/AGENTS.md or .opencode/AGENTS.md, appended to system prompt |
| File watcher config | `[ ]` | `packages/core/src/config/watcher.ts` | Watcher ignore patterns |
| Formatter integration | `[ ]` | `packages/core/src/config/formatter.ts` | Post-write formatting |
| Policy system | `[ ]` | `packages/core/src/policy.ts` | Declarative allow/deny rules |
| Session todo (persistent) | `[ ]` | `packages/core/src/session/todo.ts` | DB-backed per-session todo |
| Image processing | `[ ]` | `packages/core/src/image/photon.ts` | Image resize/optimize for vision models |
| Observable (OTLP) | `[ ]` | `packages/core/src/observability/otlp.ts` | OpenTelemetry tracing |
| Structured logging | `[ ]` | `packages/core/src/observability/logging.ts` | Key=value logging |
| Background job system | `[ ]` | `packages/core/src/background-job.ts` | Job queue with status/cancel |

## P3 — Minor/Polish

| Gap | Status | Original location | Notes |
|-----|--------|-------------------|-------|
| CLI subcommands | `[~]` | `packages/cli/src/commands/` | Missing: `migrate`, `debug agents`, `service {start,stop,restart,status,password}` |
| Serve flags | `[~]` | `packages/cli/src/commands/handlers/serve.ts` | Missing: `--hostname`, `--register` |
| Plugin SDK | `[ ]` | `packages/sdk/js/` | Build plugins with authenticated server comms |
| Migration (v1→v2) | `[ ]` | `packages/cli/src/commands/handlers/migrate.ts` | Data migration tooling |
| Version detection | `[x]` | `packages/core/src/installation/version.ts` | `--version` flag, `/version` command |
| Global paths | `[ ]` | `packages/core/src/global.ts` | XDG-based directory resolution |
| Locale utilities | `[ ]` | `packages/tui/src/util/locale.ts` | Text truncation, number formatting |
| Presentation utilities | `[ ]` | `packages/tui/src/util/presentation.ts` | Session epilogue formatting |
| Transcript formatting | `[ ]` | `packages/tui/src/util/transcript.ts` | Session export formatting |
| File type detection | `[ ]` | `packages/tui/src/util/filetype.ts` | Extension→display name |
| Path utilities | `[ ]` | `packages/tui/src/util/path.ts` | Path normalization |
| Format utilities | `[ ]` | `packages/tui/src/util/format.ts` | Duration formatting |
| Tool display names | `[x]` | `packages/tui/src/util/tool-display.ts` | Human-readable names for all 13 tools |

---

## Legend
- `[-]` out of scope (intentionally not ported)
- `[ ]` not started
- `[~]` in progress / partially done
- `[x]` done
