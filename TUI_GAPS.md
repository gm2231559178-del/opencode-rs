# TUI Design Gaps — opencode-rs vs opencode

> Generated 2026-06-14

## Legend
- `[ ]` not started
- `[~]` in progress
- `[x]` done
- `[-]` ignore

---

## P0 — Theme System (25 missing tokens, 27 missing themes)

### Color Tokens

- `[ ]` Add `info` — informational status color (used in toasts, status indicators)
- `[ ]` Add `selectedListItemText` — text color for selected list items
- `[ ]` Add `backgroundMenu` — menu/dropdown background
- `[ ]` Add `borderSubtle` — subtle/secondary border color

### Diff Tokens (11 needed, 3 exist)

- `[ ]` Add `diffContext` — context line foreground
- `[ ]` Add `diffHighlightAdded` — highlighted added line fg
- `[ ]` Add `diffHighlightRemoved` — highlighted removed line fg
- `[ ]` Add `diffAddedBg` — added line background
- `[ ]` Add `diffRemovedBg` — removed line background
- `[ ]` Add `diffContextBg` — context line background
- `[ ]` Add `diffLineNumber` — line number color
- `[ ]` Add `diffAddedLineNumberBg` — added line number background
- `[ ]` Add `diffRemovedLineNumberBg` — removed line number background

### Markdown Tokens (15 needed, 0 exist)

- `[ ]` Add `markdownText` — body text
- `[ ]` Add `markdownHeading` — headings (h1-h6)
- `[ ]` Add `markdownLink` — link underline/url
- `[ ]` Add `markdownLinkText` — link label text
- `[ ]` Add `markdownCode` — inline code spans
- `[ ]` Add `markdownBlockQuote` — block quote bars
- `[ ]` Add `markdownEmph` — italic/emphasis
- `[ ]` Add `markdownStrong` — bold/strong
- `[ ]` Add `markdownHorizontalRule` — horizontal rule characters
- `[ ]` Add `markdownListItem` — list item markers (-, *, +)
- `[ ]` Add `markdownListEnumeration` — numbered list digits
- `[ ]` Add `markdownImage` — image brackets
- `[ ]` Add `markdownImageText` — image alt text
- `[ ]` Add `markdownCodeBlock` — code block text (vs inline code)

### Syntax Tokens (5 missing, 4 exist)

- `[ ]` Add `syntaxFunction` — function/method names
- `[ ]` Add `syntaxVariable` — variable identifiers
- `[ ]` Add `syntaxType` — type/class names
- `[ ]` Add `syntaxOperator` — operators (+, -, &&, etc.)
- `[ ]` Add `syntaxPunctuation` — delimiters/brackets

### More Themes

- `[ ]` Load themes from JSON files (matching original TS format) instead of hardcoding
- `[ ]` Add 27 missing themes (aura, ayu, carbonfox, catppuccin-frappe, catppuccin-macchiato, cobalt2, cursor, everforest, flexoki, github, kanagawa, lucent-orng, material, matrix, mercury, monokai, nightowl, opencode, orng, osaka-jade, palenight, rosepine, solarized, synthwave84, vercel, vesper, zenburn)
- `[ ]` Add `thinkingOpacity` non-color config field

---

## P1 — Diff Viewer

- `[ ]` **Inline diff rendering** — render diffs inside the message flow (not just full-screen overlay)
- `[ ]` **Split/unified view toggle** — add `diff_style` config (auto/stacked) and keybinding toggle
- `[ ]` **File tree sidebar** — directory hierarchy for multi-file diffs
- `[ ]` **Review marking** — `m` key to mute/mark files as reviewed
- `[ ]` **Hunk navigation** — `[` / `]` jumps between diff hunks
- `[ ]` **File navigation** — `n` / `p` cycles through files
- `[ ]` **Source switching** — toggle between working tree and last turn diffs
- `[ ]` **Per-type line number backgrounds** — use diff added/removed line number bg colors
- `[ ]` **Line background colors** — green/red tint backgrounds for added/removed lines
- `[ ]` **Wrap mode config** — configurable word/char wrap for diffs

---

## P1 — Syntax Highlighting & Code Display

- `[ ]` Increase language coverage from 6 families to 20+ (add Ruby, PHP, Swift, Kotlin, Scala, Rust, SQL, YAML, TOML, JSON, HTML, CSS, shell)
- `[ ]` Expand keyword lists per language (use tree-sitter grammar data where possible)
- `[ ]` Render inline code spans with `markdownCode` color
- `[ ]` Render block quotes with `markdownBlockQuote` vertical bar
- `[ ]` Render headings with `markdownHeading` color + bold
- `[ ]` Render links with `markdownLink` underline color + `markdownLinkText` label color
- `[ ]` Render strong/emph with proper styling
- `[ ]` Render list markers with `markdownListItem` / `markdownListEnumeration`
- `[ ]` Render horizontal rules with `markdownHorizontalRule`

---

## P2 — Logo / Splash / Background

- `[ ]` Add ASCII logo on startup (open code "GO" logo)
- `[ ]` Add idle shimmer / concentric ring animation on logo
- `[ ]` Add bg-pulse effect (animated ring waves with breathing)
- `[ ]` Sub-pixel rendering via `▀`/`▄` half-block characters for double vertical resolution
- `[ ]` Frame caching for animation performance
- `[ ]` Global animation toggle (`app.toggle.animations`)

---

## P2 — Audio & Notifications

- `[-]` Sound effects on events (question asked, permission needed, error, done) — intentionally skipped, terminal TUI has no native audio API
- `[-]` Configurable sound packs (pluggable) — intentionally skipped
- `[-]` Focus-aware delivery (only when terminal is focused/blurred) — intentionally skipped
- `[-]` Toast notification variants (success/error/info/warning) — intentionally skipped
- `[-]` Enable desktop notifications by default (via notify-rush) — intentionally skipped
- `[-]` Terminal bell fallback for audio — intentionally skipped

---

## P2 — Autocomplete

- `[ ]` Frecency ranking (sort by frequency + recency of selection)
- `[ ]` MCP tool autocomplete candidates
- `[ ]` `#L` line range suffix on `@` file references
- `[ ]` Type icons in autocomplete popup (file, ref, command, MCP)

---

## P2 — TUI Plugin System

- `[ ]` Plugin slots for sidebar panels
- `[ ]` Plugin API for custom dialogs
- `[ ]` Plugin keybinding contributions
- `[ ]` Plugin footer contributions
- `[ ]` Custom command registration via plugins

---

## P3 — Scroll & Navigation

- `[ ]` Momentum scroll acceleration (configurable on/off)
- `[ ]` Configurable scroll speed
- `[ ]` Themed scrollbar visualization (track + thumb)

---

## P3 — Input / Prompt Polish

- `[ ]` Placeholder text when input is empty
- `[ ]` Character count / buffer status indicator
- `[ ]` Separate metadata footer row below input (instead of merged inside input box)

---

## P3 — Info & Status

- `[ ]` Session epilogue on close (formatted summary)
- `[ ]` Transcript export formatting
- `[ ]` Share dialog with QR code display
- `[ ]` Visual token usage charts/graphs in status dialog

---

## P4 — Animations & Transitions

- `[ ]` Fade-in animation for new messages (smoothstep alpha ramp over 160ms)
- `[ ]` Global animation enable/disable config

---

## Files to modify

| File | What to change |
|------|----------------|
| `src/theme.rs` | Add ~25 color tokens, add `thinkingOpacity`, load from JSON |
| `src/tui.rs` | Diff viewer overhaul, markdown rendering, syntax highlighting, logo, autocomplete, input polish, scroll acceleration, animations |
| `src/util/filetype.rs` | Expand language coverage + keyword lists |
| `src/util/locale.rs` | Add `format_number`, `format_duration` (missing) |
| `src/session.rs` | Session epilogue/transcript formatting |
| `src/config.rs` | Add `diff_style`, `scroll_speed`, `animations_enabled`, audio config |
| `src/plugin.rs` | TUI plugin slots |
| New: `src/tui/logo.rs` | Logo rendering logic |
| New: `src/tui/bg_pulse.rs` | Background animation engine |
| New: `src/tui/audio.rs` | Audio/attention system |
| New: `src/util/presentation.rs` | Session epilogue formatting |
| New: `src/util/transcript.rs` | Transcript export formatting |
