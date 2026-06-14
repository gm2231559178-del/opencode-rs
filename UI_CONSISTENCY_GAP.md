# UI Consistency Gap Analysis: opencode-rs vs opencode

> Analysis of Terminal UI design consistency gaps between the opencode-rs Rust TUI (this project) and the opencode TypeScript TUI (target reference).

---

## 1. Visual Design System

### 1.1 Color Tokens

| Aspect | opencode-rs | opencode | Gap |
|--------|------------|----------|-----|
| Token count | ~56 tokens | ~90 tokens | **Missing 34 tokens** — no `info` semantic color, incomplete markdown (14 missing) and syntax tokens, only 4 background levels vs richer hierarchy |
| `selectedListItemText` | Defined but **only used in diff viewer** (`src/tui.rs:2729`), **not in select dialogs** which use `fg(t.bg).bg(t.primary)` | Used consistently across all selection contexts | **Internal inconsistency** — token exists but is ignored by primary list components |
| `thinkingOpacity` | Defined (`src/theme.rs:76`) but **never referenced in rendering code** | Used by `generateSubtleSyntax()` for dimmed reasoning display | **Dead token** — declared but has no behavioral effect |
| `info` semantic | ❌ Not present | ✅ Core semantic color | Missing status color for neutral informational states |
| Background hierarchy | 4 levels — but `background_menu` only used in footer (`src/tui.rs:2843`), `background_element` usage varies between dialogs and panels | 4 levels with clear semantics: `bg` (transparent base), `backgroundPanel` (secondary), `backgroundElement` (hover/active), `backgroundMenu` (popovers) | **Shallow usage** — hierarchy exists but surfaces aren't consistently applied across components |

### 1.2 Selection Highlight Convention

| Component | opencode-rs | opencode |
|-----------|------------|----------|
| Select dialogs | `fg(t.bg).bg(t.primary)` — dark text on primary | `backgroundColor={theme.primary}` with `fg=selectedListItemText` |
| Autocomplete popup | `fg(t.text).bg(t.accent)` — **different token** | Same convention as dialogs |
| Diff file sidebar | `selectedListItemText` foreground | Same as dialogs |
| Category headers | `t.accent, BOLD` | `theme.accent, BOLD` |

**Inconsistency in opencode-rs**: 3 different selection highlight conventions across 3 components. The `selectedListItemText` token (`src/theme.rs:74`) is only used in the diff viewer sidebar, not in the primary list components.

### 1.3 Border Patterns

| Aspect | opencode-rs | opencode |
|--------|------------|----------|
| Dialog borders | `Borders::ALL` with `primary` color | Full border boxes with consistent `SplitBorder` (┃) on left for panels |
| Content panels | Input: `Borders::LEFT`, Sidebar: `Borders::RIGHT` | Universal `SplitBorder` (┃ left+right) for messages, tools, panels |
| Reusable primitive | ❌ None — each component specifies `Borders::*` independently | ✅ `EmptyBorder` base + `SplitBorder` composed pattern reused across all panels, messages, tools, prompts, toasts |

---

## 2. Spacing & Layout

### 2.1 Spacing System

| Aspect | opencode-rs | opencode |
|--------|------------|----------|
| Approach | Ad-hoc: hardcoded `"  "` strings, `Line::from("")`, `saturating_sub()` | Consistent `paddingLeft={N}`, `paddingRight={N}`, `gap={N}` patterns |
| Content padding | `w.saturating_sub(4)` for messages, `w.saturating_sub(2)` for code blocks — **different values per component** | Uniform `paddingLeft={3}` for text/reasoning parts, `paddingLeft={2}` for dialogs |
| Vertical spacing | `Line::from("")` blank lines throughout | `marginTop={1}` between messages, `gap={1}` in flex containers |
| **Gap** | **No centralized spacing constants** — every component calculates its own padding | Consistent inner padding: dialogs=2, content items=3, sidebar=2 |

### 2.2 Dialog Positioning

| Aspect | opencode-rs | opencode |
|--------|------------|----------|
| Width | Varies: 50 (confirm), 60 (help/alert/workspace/prompt), 70 (select) | Three standard sizes: medium=60, large=88, xlarge=116 |
| Vertical position | 1/3 from top (hardcoded) | 1/4 from top (`height/4`) |
| Horizontal | Centered | Centered with 2-col margin (`maxWidth=width-2`) |
| Backdrop | `Clear` widget (transparent — underlying content visible) | `RGBA(0,0,0,150)` semi-transparent dark overlay |
| **Gap** | **No standard dialog sizing**; no backdrop dimming; different width per dialog type breaks visual rhythm | Consistent size tiers with identical backdrop + positioning |

### 2.3 Internal Positioning Divergence

`src/tui.rs` has two positioning functions that behave differently:

- **`dialog_area()`** (`tui.rs:3973`): caps at 80×40, used by help/alert/confirm/workspace dialogs
- **`centered_rect()`** (`tui.rs:3981`): no capping, caller specifies dimensions, used by select/prompt dialogs

This creates subtle inconsistency — some dialogs have a maximum size, others don't.

---

## 3. Component Consistency

### 3.1 Status Indicators

| Pattern | opencode-rs | opencode |
|---------|------------|----------|
| Connected | `•` green dot (sidebar MCP, `tui.rs:2506`) | `●` green dot (footer) |
| Error | `•` red dot (sidebar MCP, `tui.rs:2507`) | `●` red dot OR `⊙` symbol for MCP |
| Warning | `•` yellow dot (`needs_auth`) | `△` symbol for permissions |
| Idle/off | `•` dim colored | `○` hollow circle |
| **Gap** | **No hollow/unfilled variant** (all `•`); **no permission indicator** | Richer symbol set with distinct visual meanings |

### 3.2 Toast/Snackbar

| Aspect | opencode-rs | opencode |
|--------|------------|----------|
| Position | Bottom-right (`tui.rs:2775`) | Top-right |
| Duration | 6 frames (~100ms) — **too short to read** | 5000ms default |
| Variants | All `t.success` (green) — **single variant** | 4 variants: `info`, `success`, `warning`, `error` with matching border |
| Structure | Plain text only | Title (bold) + Message (wrap) |
| History | ❌ None | ✅ 50-entry history with dialog viewer |
| **Gap** | **Single variant, no history, unusably short duration** | Rich variant system with history |

### 3.3 Message Rendering

| Aspect | opencode-rs | opencode |
|--------|------------|----------|
| Left border | `▎` marker bar in role color + role indicator (`⚙`/`↳`) for tool messages | `SplitBorder` (┃) on left |
| Background per role | 2 variants: `backgroundPanel` (user) vs `bg` (all others) | Consistent `backgroundPanel`, `backgroundElement` on hover |
| Spacing | Margin via blank `▎` line between messages | `marginTop={1}`, `marginTop={0}` for first |
| Collapse behavior | Long tool results (>200 chars) auto-collapsed at creation with smart preview | Default collapsed, expand on click |
| Role indicators | `⚙` / `↳` symbols in left border for tool_call / tool_result | Inline header labels |
| Role colors | 5 distinct border colors | Border colored by agent color (user), muted (assistant) |
| File attachments | Plain text inline | Two-tone pill badges with type + filename |
| Tool variants | Uniform rendering for all tools | 15+ tool-specific icons/layouts/expand patterns |
| Hover effects | ❌ None | ✅ `backgroundElement` / `backgroundMenu` on hover |

---

## 4. Interaction Patterns

### 4.1 Keyboard Dismissal

| Aspect | opencode-rs | opencode |
|--------|------------|----------|
| Close dialog | `Esc` | `Escape` or `Ctrl+C` |
| Dismiss toast | Auto (6 frames) | Auto (5s) or click |
| Cancel streaming | `Esc` | `Escape` (double-tap within 5s) |
| Selection guard | ❌ None — dialogs close even while selecting | ✅ First `Escape` clears selection before dismissing |
| Quit keys | `Ctrl+C` / `q` — **why both?** | Single convention (`Ctrl+C` only) |

### 4.2 Hover/Focus States

| Aspect | opencode-rs | opencode |
|--------|------------|----------|
| Mouse tracking | ❌ Not supported (ratatui) | ✅ `onMouseMove`, keyboard vs mouse mode detection |
| Focus indication | `BOLD` titles, `border_active` borders | `backgroundColor` shifts: `backgroundElement` or `backgroundMenu` |
| Active element | `fg(t.bg).bg(t.primary)` dialogs, `fg(t.text).bg(t.accent)` autocomplete | `backgroundColor={theme.primary}` consistently |

### 4.3 Scroll Patterns

| Aspect | opencode-rs | opencode |
|--------|------------|----------|
| Acceleration | Custom: `scroll_speed × min(accel, 8)` | Configurable via `getScrollAcceleration()` |
| Scrollbar | Custom `░` track + `██` thumb rendered manually | Consistent `trackOptions` with theme colors |
| Scroll indicator | `"↑ N more"` text (`tui.rs:3035`) | Inline indicators within scrollable areas |

---

## 5. Footer & Status Bar

| Aspect | opencode-rs | opencode |
|--------|------------|----------|
| Structure | Single line: `PLAN │ LEADER │ agent │ model│ status | ctx%` | Two-part: session footer (path + status dots) + prompt footer (agent/model/provider/variant + cost) |
| Separators | `│` in `t.border` | `·` in muted |
| Status icons | Symbols: `●`/`○` for streaming/idle + `△ perm` for pending permission | Symbol icons: `●`/`○`/`△`/`⊙` |
| Background | `t.background_menu` | Theme default |
| **Gap** | **Single-line overloaded bar (narrow screen issue)**; missing `⊙` MCP symbol | Dual-footer with richer iconography |

---

## 6. Inconsistencies Within opencode-rs

1. ~~**Selection highlight**: 3 conventions across 3 components~~ ✅ Unified to `selectedListItemText` on `primary` background
2. ~~**Toast duration**: `show_toast()` sets 6 frames, but auto-compact toast hardcodes 80 frames~~ ✅ Standardized with `TOAST_DURATION_*` constants (30 normal, 60 error, 80 long)
3. ~~**Dialog widths**: Vary 50–70 without semantic reason~~ ✅ Standardized to 60 (`DIALOG_WIDTH`)
4. ~~**`dialog_area` vs `centered_rect`**~~ ✅ `dialog_area` now delegates to `centered_rect`
5. ~~**`thinking_opacity`**: Declared but never used~~ ✅ Wired up via `Modifier::DIM` for reasoning messages
6. ~~**`selectedListItemText`**: Defined but only used in diff viewer~~ ✅ Now used in select dialogs and autocomplete
7. ~~**Quit keys ambiguity**: Both `Ctrl+C` AND `q`~~ ✅ Removed `q` quit — only `Ctrl+C`

---

## 7. Summary of Priority Actions

| Priority | Gap | Impact | Status |
|----------|-----|--------|--------|
| **Critical** | No unified selection highlight convention | Users see different visual feedback in different list contexts | ✅ Fixed |
| **Critical** | Toast has single variant, no history, too short | Users miss notifications, can't distinguish error from success | ✅ Fixed |
| **High** | Dialog widths vary arbitrarily (50/60/70) | Visual rhythm broken when switching dialog types | ✅ Fixed |
| **High** | No backdrop dimming for dialogs | Overlapping content causes visual noise | ✅ Fixed |
| **High** | No hover/focus state differentiation | Users can't identify interactive vs static elements | ❌ Open (requires mouse infra) |
| **High** | Missing `info` semantic + `permission` indicator | Status reporting is incomplete | ✅ Fixed |
| **Medium** | Footer overloads single line, lacks semantic icons | Information hierarchy unclear | ✅ Fixed |
| **Medium** | No spacing constants — ad-hoc throughout | Layout breaks unpredictably at different terminal sizes | ✅ Fixed |
| **Medium** | `selectedListItemText` token ignored by dialogs | Theme token exists but unused in primary selection | ✅ Fixed |
| **Medium** | No spacing between messages | Cluttered output, hard to visually separate messages | ✅ Fixed |
| **Medium** | No tool role indicators in left border | All roles look similar with just bar color difference | ✅ Fixed |
| **Medium** | Long tool results always expanded | Long outputs forced in view, no smart default | ✅ Fixed |
| **Low** | `thinking_opacity` is dead code | Theme bloat with no behavioral effect | ✅ Fixed |
| **Low** | `dialog_area` vs `centered_rect` divergence | Minor positioning inconsistency on small terminals | ✅ Fixed |
| **Low** | Redundant `streaming`/`idle` text in footer | Symbol already conveys state, text is noise | ✅ Fixed |
