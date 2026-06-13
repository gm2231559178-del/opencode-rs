# UI Differences: Rust TUI vs Original TypeScript TUI

> Generated 2026-06-13 — Visual and layout gaps between `/project/workspace/src/tui.rs` (Rust/ratatui) and `/project/opencode/packages/tui/src/` (TypeScript/@opentui/solid).

## Layout & Structure

| Gap | Original | Rust | Impact |
|-----|----------|------|--------|
| **Sidebar position** | Right side, 42 chars | Left side, 36 chars | High — changes left-to-right reading flow |
| **"Chat" header** | None above messages | `Borders::TOP` with title `" Chat "` | High — extra visual element not in original |
| **Status bar** | Inside the prompt/input border box | Separate full-width row between messages and input | Medium — changes the bottom bar grouping |
| **Message direction** | Oldest at top, newest at bottom (fixed) | Oldest at top, newest at bottom | Fixed in f90ea0e |
| **Input bar** | Left-accent border, textarea + metadata footer row | `Borders::ALL` block with contextual title (Leader/Approve/Input) | Medium |
| **Autocomplete popup** | Overlaid above the prompt, anchored to input | Inline list row between status and input bar, `Borders::ALL` | Low |

## Message Rendering

| Gap | Original | Rust | Impact |
|-----|----------|------|--------|
| **Role labels** | None — no "user"/"assistant" badges | Bold colored "user", "assistant", "think", "tool", "result" labels | High — extra text in message headers |
| **User messages** | Left border only, no label, `background_panel` bg | `▎` bar + bold "user" label, `background_element` bg | High |
| **Assistant messages** | No background (default), text parts at `paddingLeft=3` | `background_panel` bg, role-colored `▎` bar | Medium |
| **Tool call icons** | Per-tool type: `$` shell, `✱` glob, `→` read, `←` write, `✓` done, `⚙` generic, `%` fetch, `◈` search | Plain text `tool_name (short_id)` | Medium |
| **Message spacing** | `marginTop=1` between message blocks | Empty `Line::from("")` appended to each message | Low |
| **Timestamps** | Optional per-message timestamp (`showTimestamps()`) | None | Low |

## Scroll & Navigation

| Gap | Original | Rust | Impact |
|-----|----------|------|--------|
| **Scrollbar** | Toggleable themed scrollbar (track/foreground colors) | None — PageUp/PageDown only | Medium |
| **Sticky scroll** | `stickyStart="bottom"` — auto-follows newest | Manual scroll, resets to newest on new message | Low |

## Feature Rendering

| Gap | Original | Rust | Impact |
|-----|----------|------|--------|
| **Code blocks** | Full markdown syntax highlighting (language-specific tokens) | Fence detection only, uniform dim style, diff lines colored | Medium |
| **Reasoning/thinking** | Per-block spinner, collapse/expand, duration display | Global toggle (`reasoning_visible`), no spinner/duration | Low |
| **Diff display** | Inline `<diff>` component in message flow (line numbers, split/unified) | Separate full-screen overlay (`/diff`) | Low |
| **Toast notifications** | Stacked bottom-right overlay, variants (success/error/info/warning) | Full-width row between messages and status bar, single style | Low |

## Infrastructure

| Gap | Original | Rust | Impact |
|-----|----------|------|--------|
| **Theme colors** | 40+ color tokens (scrollbar, audio, secondary agent tags, etc.) | 20 tokens | Low |
| **Message backgrounds** | User: `background_panel`; Assistant: none | User: `background_element`; Assistant: `background_panel` | Low |
| **Tool output collapse** | Per-tool max-lines config, click-to-expand | Global `Ctrl+O`, 100-char preview with `[+N chars]` | Low |
| **File type detection** | `filetype.ts` — 120+ extension→language map | None | Low |
| **Locale utilities** | `truncateMiddle`, number formatting | None | Low |
