# UI Differences: Rust TUI vs Original TypeScript TUI

> Generated 2026-06-13 — Visual and layout gaps between `/project/workspace/src/tui.rs` (Rust/ratatui) and `/project/opencode/packages/tui/src/` (TypeScript/@opentui/solid).

## Layout & Structure

| Gap | Original | Rust | Status |
|-----|----------|------|--------|
| **Sidebar position** | Right side, 42 chars | Right side, 42 chars | Fixed |
| **"Chat" header** | None above messages | None | Fixed |
| **Status bar** | Inside the prompt/input border box | Inside the input box, no top border | Fixed |
| **Message direction** | Oldest at top, newest at bottom | Oldest at top, newest at bottom | Fixed |
| **Input bar** | Left-accent border, textarea + metadata footer row | `Borders::LEFT` border, no title | Fixed |
| **Autocomplete popup** | Overlaid above the prompt, anchored to input | Overlaid above input bar, `Borders::ALL` | Fixed |

## Message Rendering

| Gap | Original | Rust | Status |
|-----|----------|------|--------|
| **Role labels** | None — no "user"/"assistant" badges | None | Fixed |
| **User messages** | Left border only, no label, `background_panel` bg | `▎` bar only, `background_panel` bg | Fixed |
| **Assistant messages** | No background (default), text parts at `paddingLeft=3` | No background, `▎` bar | Fixed |
| **Tool call icons** | Per-tool type: `$` shell, `✱` glob, `→` read, `←` write, `✓` done, `⚙` generic, `%` fetch, `◈` search | Per-tool icons with human name | Fixed |
| **Message spacing** | `marginTop=1` between message blocks | Empty `Line::from("")` appended to each message | Open |
| **Timestamps** | Optional per-message timestamp (`showTimestamps()`) | None | Open |

## Scroll & Navigation

| Gap | Original | Rust | Status |
|-----|----------|------|--------|
| **Scrollbar** | Toggleable themed scrollbar (track/foreground colors) | `↑ N` text indicator when scrolled | Fixed |
| **Sticky scroll** | `stickyStart="bottom"` — auto-follows newest | Manual scroll, resets to newest on new message | Open |

## Feature Rendering

| Gap | Original | Rust | Status |
|-----|----------|------|--------|
| **Code blocks** | Full markdown syntax highlighting (language-specific tokens) | Fence detection only, uniform dim style, diff lines colored | Open |
| **Reasoning/thinking** | Per-block spinner, collapse/expand, duration display | Global toggle (`reasoning_visible`), no spinner/duration | Open |
| **Diff display** | Inline `<diff>` component in message flow (line numbers, split/unified) | Separate full-screen overlay (`/diff`) | Open |
| **Toast notifications** | Stacked bottom-right overlay, variants (success/error/info/warning) | Bottom-right overlay, success color | Fixed |

## Infrastructure

| Gap | Original | Rust | Status |
|-----|----------|------|--------|
| **Theme colors** | 40+ color tokens (scrollbar, audio, secondary agent tags, etc.) | 20 tokens | Open |
| **Message backgrounds** | User: `background_panel`; Assistant: none | User: `background_panel`; Assistant: none | Fixed |
| **Tool output collapse** | Per-tool max-lines config, click-to-expand | Global `Ctrl+O`, 100-char preview with `[+N chars]` | Open |
| **File type detection** | `filetype.ts` — 120+ extension→language map | None | Open |
| **Locale utilities** | `truncateMiddle`, number formatting | None | Open |
