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
| **Message spacing** | `marginTop=1` between message blocks | `▎` bar provides visual separation, no extra empty line | Fixed |
| **Timestamps** | Optional per-message timestamp (`showTimestamps()`) | Ctrl+T toggle, HH:MM:SS format | Fixed |

## Scroll & Navigation

| Gap | Original | Rust | Status |
|-----|----------|------|--------|
| **Scrollbar** | Toggleable themed scrollbar (track/foreground colors) | `↑ N` text indicator when scrolled | Fixed |
| **Sticky scroll** | `stickyStart="bottom"` — auto-follows newest | Auto-follows when at bottom, keeps place when scrolled up | Fixed |

## Feature Rendering

| Gap | Original | Rust | Status |
|-----|----------|------|--------|
| **Code blocks** | Full markdown syntax highlighting (language-specific tokens) | Language-aware highlighting for Rust/Python/JS/Go/Java/C++ | Fixed |
| **Reasoning/thinking** | Per-block spinner, collapse/expand, duration display | Spinner on reasoning blocks during streaming, global visibility toggle | Fixed |
| **Diff display** | Inline `<diff>` component in message flow (line numbers, split/unified) | Full-screen overlay with line numbers, colored +/- | Fixed |
| **Toast notifications** | Stacked bottom-right overlay, variants (success/error/info/warning) | Bottom-right overlay, success color | Fixed |

## Infrastructure

| Gap | Original | Rust | Status |
|-----|----------|------|--------|
| **Theme colors** | 40+ color tokens (scrollbar, audio, secondary agent tags, etc.) | 28 tokens (added diff, syntax colors) | Fixed |
| **Message backgrounds** | User: `background_panel`; Assistant: none | User: `background_panel`; Assistant: none | Fixed |
| **Tool output collapse** | Per-tool max-lines config, click-to-expand | Global toggle for all tool messages, preview | Fixed |
| **File type detection** | `filetype.ts` — 120+ extension→language map | 80+ extension→language map + normalize_language() | Fixed |
| **Locale utilities** | `truncateMiddle`, number formatting | `truncate_middle`, `format_number`, `format_duration` | Fixed |
