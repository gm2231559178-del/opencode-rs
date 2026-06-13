# Remaining UI Gaps — Analysis

Generated 2026-06-13. Analysis of the 6 remaining open gaps between the Rust TUI and the original TypeScript TUI.

---

## 1. Theme Colors (Medium effort, High impact)

**Context:**
- Rust `Theme` struct has 23 fields (20 color tokens + name).
- Original defines 40+ tokens including diff colors, markdown colors, syntax colors, and UI tokens.
- Currently reuses generic tokens: `diff_add` uses `theme.success`, syntax keywords use `theme.accent`, etc.

**What would change:**
- Add ~20-25 fields to `Theme` in `src/theme.rs`
- Update all 6 theme constants (TOKYONIGHT, CATPPUCCIN, etc.)
- Update rendering in `tui.rs` to use new specific tokens

**Files:** `src/theme.rs`, `src/tui.rs`

---

## 2. Timestamps (Small effort, Medium impact)

**Context:**
- `TuiMessage` has `role`, `content`, `age` — no timestamp.
- `chrono` already a dependency. Original has `session_toggle_timestamps` keybinding.
- `age` field exists but is not a real timestamp.

**What would change:**
- Add `timestamp: chrono::DateTime<chrono::Utc>` to `TuiMessage`
- Set at ~16 creation sites in `tui.rs`
- Add `show_timestamps: bool` toggle + keybinding
- Render `HH:MM` format alongside messages

**Files:** `src/tui.rs`

---

## 3. File Type Detection (Small-Medium effort, Medium impact)

**Context:**
- Rust syntax highlighting supports 6 language families (Rust, Go, Python, JS/TS, Java, C/C++).
- Original `filetype.ts` maps 120+ extensions to ~70 language names.
- `LspServer::language_for_file` has a 12-entry map for LSP.
- Highlighting infrastructure already handles arbitrary language strings; just need more keyword/comment data.

**What would change:**
- Create `src/util/filetype.rs` with 120+ extension→language mapping
- Add keyword lists, comment info, and builtins for 10-15 more languages

**Files:** New `src/util/filetype.rs`, `src/util/mod.rs`, `src/tui.rs`

---

## 4. Diff Inline (Medium-Large effort, Medium impact)

**Context:**
- Original renders `<diff>` inline in message flow with line numbers.
- Rust has separate full-screen overlay (`/diff` command) with colored +/- entries.
- Diffs are also colored inline inside code blocks in `render_highlighted`.

**What would change:**
- Requires structured content parsing within messages (not just markdown code fences).
- Alternative: enhance full-screen viewer with line numbers, split/unified toggle.

**Files:** `src/tui.rs`, `src/session.rs`, potentially new diff rendering module

---

## 5. Tool Output Per-Tool Collapse (Small effort, Medium impact)

**Context:**
- Original: configurable `maxLines`/`maxChars` per tool, click-to-expand.
- Rust: Global `Ctrl+O` toggles collapse on the last tool result with 100-char preview.
- `collapsed: HashSet<usize>` already tracks state per message index.

**What would change:**
- Main change: allow selecting individual tool messages to toggle collapse (needs message selection UX).
- Adjust truncation parameters to be configurable.
- Show `[+N lines]` instead of `[+N chars]`.

**Files:** `src/tui.rs`

---

## 6. Locale Utilities (Very small effort, Low impact)

**Context:**
- Original: `truncateMiddle`, `truncateLeft`, `number()` (1.2K, 3.5M), `duration()`, `time()`, `datetime()`, `pluralize()`.
- Rust uses `textwrap::fill` and ad-hoc `chars().take()` truncation.
- No compact number formatting anywhere.

**What would change:**
- Create `src/util/locale.rs` with `truncate_middle`, `truncate_left`, `format_number`, `format_duration`.

**Files:** New `src/util/locale.rs`, `src/util/mod.rs`

---

## Recommendation (by effort/impact ratio)

| Rank | Gap | Effort | Impact |
|------|-----|--------|--------|
| 1 | Theme Colors | Medium | High |
| 2 | Timestamps | Small | Medium |
| 3 | File Type Detection | Small-Medium | Medium |
| 4 | Diff Inline | Medium-Large | Medium |
| 5 | Tool Output Per-Tool | Small | Medium |
| 6 | Locale Utilities | Very Small | Low |
