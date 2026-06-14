# TUI Design Gaps — opencode-rs vs opencode

> Updated 2026-06-14

## Legend
- `[ ]` not started | `[~]` in progress | `[x]` done | `[-]` intentionally skipped

---

## P0 — Theme System (25 missing tokens, 27 missing themes)

### Color Tokens (all added to struct + 7 themes)

- `[x]` Add `info` — informational status color
- `[x]` Add `selectedListItemText` — text color for selected list items
- `[x]` Add `backgroundMenu` — menu/dropdown background
- `[x]` Add `borderSubtle` — subtle/secondary border color

### Diff Tokens (11 needed, 3 existed → 12 now)

- `[x]` Add `diffContext` — context line foreground
- `[x]` Add `diffHighlightAdded` — highlighted added line fg
- `[x]` Add `diffHighlightRemoved` — highlighted removed line fg
- `[x]` Add `diffAddedBg` — added line background
- `[x]` Add `diffRemovedBg` — removed line background
- `[x]` Add `diffContextBg` — context line background
- `[x]` Add `diffLineNumber` — line number color
- `[x]` Add `diffAddedLineNumberBg` — added line number background
- `[x]` Add `diffRemovedLineNumberBg` — removed line number background

### Markdown Tokens (15 needed, 0 existed → 15 now)

- `[x]` Add `markdownText` — body text
- `[x]` Add `markdownHeading` — headings
- `[x]` Add `markdownLink` — link underline/url
- `[x]` Add `markdownLinkText` — link label text
- `[x]` Add `markdownCode` — inline code spans
- `[x]` Add `markdownBlockQuote` — block quote bars
- `[x]` Add `markdownEmph` — italic/emphasis
- `[x]` Add `markdownStrong` — bold/strong
- `[x]` Add `markdownHorizontalRule` — horizontal rule
- `[x]` Add `markdownListItem` — list item markers
- `[x]` Add `markdownListEnumeration` — numbered list digits
- `[x]` Add `markdownImage` — image brackets
- `[x]` Add `markdownImageText` — image alt text
- `[x]` Add `markdownCodeBlock` — code block text

### Syntax Tokens (5 missing, 4 existed → 9 now)

- `[x]` Add `syntaxFunction` — function/method names
- `[x]` Add `syntaxVariable` — variable identifiers
- `[x]` Add `syntaxType` — type/class names
- `[x]` Add `syntaxOperator` — operators
- `[x]` Add `syntaxPunctuation` — delimiters/brackets

### More Themes

- `[x`] Load themes from JSON files (34 themes from `themes/` directory)
- `[x]` Add 27 missing themes via JSON files (copied from original project format)
- `[x]` Add `thinkingOpacity` non-color config field

---

## P1 — Diff Viewer

- `[x]` **Line background colors** — green/red tint backgrounds for added/removed lines
- `[x]` **Hunk navigation** — `[` / `]` jumps between diff hunks
- `[x]` **Per-type line number backgrounds** — use diff added/removed line number bg colors
- `[x]` **Status bar** — keyboard shortcut hints at bottom
- `[x]` **Inline diff rendering** — render diffs inside the message flow (not just full-screen overlay)
- `[x]` **Split/unified view toggle** — add `diff_style` config and `v` keybinding to toggle (visual split rendering TBD)
- `[x]` **File tree sidebar** — directory hierarchy for multi-file diffs
- `[x]` **Review marking** — `m` key to mute/mark files as reviewed
- `[ ]` **File navigation** — `n` / `p` cycles through files
- `[x]` **Source switching** — toggle between working tree and last turn diffs
- `[ ]` **Wrap mode config** — configurable word/char wrap for diffs

---

## P1 — Syntax Highlighting & Markdown Display

- `[x]` Increase language coverage from 6 families to 20+ (added bash, sql, perl, and improved all existing)
- `[x]` Expand keyword lists per language (bash, sql, perl, dockerfile, cmake, gradle, etc.)
- `[x]` Add type highlighting via `get_types()` function
- `[x]` Render headings with `markdownHeading` color + bold
- `[x]` Render block quotes with `markdownBlockQuote` vertical bar
- `[x]` Render inline code with `markdownCode` color
- `[x]` Render bold with `markdownStrong` style
- `[x]` Render italic with `markdownEmph` style
- `[x]` Render horizontal rules with `markdownHorizontalRule`
- `[x]` Render list markers with `markdownListItem` / `markdownListEnumeration`
- `[x]` Render links with rendered `markdownLink` + `markdownLinkText` (inline link detection)
- `[ ]` Expand language keyword lists to match tree-sitter completeness

---

## P2 — Audio & Notifications

- `[-]` All items intentionally skipped (terminal TUI has no native audio API)

---

## P2 — Autocomplete

- `[x]` Type icons in autocomplete popup (dirs: `+`, files: `>`, refs: `≡`, commands: `/`)
- `[x]` Improved file sorting (prefix matches ranked above substring)
- `[x]` Directory detection with trailing slash
- `[x]` `#L` line range suffix already supported on `@` file references (pre-existing)
- `[x]` Frecency ranking (sort by frequency of selection)
- `[ ]` MCP tool autocomplete candidates

---

## P2 — TUI Plugin System

- `[ ]` Plugin slots for sidebar panels
- `[ ]` Plugin API for custom dialogs
- `[ ]` Plugin keybinding contributions

---

## P2 — Logo / Splash / Background

- `[x]` ASCII logo on startup
- `[ ]` Frame caching for animation performance

---

## P3 — Scroll & Navigation

- `[x]` Momentum scroll acceleration (step doubles: 10→20→40→80)
- `[x]` Configurable scroll speed (`scroll_speed` setting, default 10)
- `[x]` Themed scrollbar visualization (right-edge `░` track + `██` thumb)

---

## P3 — Input / Prompt Polish

- `[x]` Placeholder text when input is empty
- `[x]` Character count in status bar
- `[x]` Separate metadata footer row below input (model/agent/tokens/status)

---

## P3 — Info & Status

- `[ ]` Session epilogue on close (formatted summary)
- `[ ]` Transcript export formatting
- `[ ]` Share dialog with QR code display

---

## P4 — Animations & Transitions

- `[x]` Age-based fade-in for new messages (DIM over 10 frames)
- `[ ]` Smoothstep easing for fade-in (currently binary DIM)
- `[ ]` Global animation enable/disable config
