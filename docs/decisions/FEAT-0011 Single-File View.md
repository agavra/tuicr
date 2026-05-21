---
title: Single-File View Mode
description: Add `:focus` toggle that renders the currently selected file alone, with cursor walking across files
type: adr
status: proposed
created: 2026-05-21
---

# FEAT-0011 Single-File View Mode

## Context and Problem Statement

Whole-repo annotation (`--all-files`, shipped in FEAT-0010) drops the user into a continuous scroll of every tracked file. On a real repo the scroll feels unbounded: the file you want is somewhere down there, the cursor's `current_file_idx` updates as you scroll, and section headers (`═══ filename ═══`) eat horizontal space when long lines need scrolling. The dominant workflow in practice is "focus one file, annotate, move on" -- the continuous view's strength (compare across files at once) is rarely what the user actually does.

This ADR introduces a sticky single-file view: render only the currently focused file in the diff panel, and walk between files by holding `j`/`k`. It does not replace the continuous view; both coexist and the user picks per session. Pane-focus navigation (Left/Right arrow keys, file-list hide/reveal) is a separate concern decided in [FEAT-0012][FEAT-0012].

## Decision Drivers

- Pristine reviews (`--all-files`) need a focus surface; continuous scroll is overwhelming for >100 tracked files.
- Diff-mode review keeps benefiting from continuous scroll (compare across files), so the new view must be a toggle, not a replacement.
- Cursor and scroll math must agree with what the renderer produces. The first single-file view PR shipped with four bugs from `total_lines`, `calculate_file_scroll_offset`, `next_hunk`, and `update_current_file_from_cursor` each branching on the mode flag in slightly different ways; the root cause was the lack of a single height helper.
- Status of which mode is active must be visible at all times (sticky mode without indicator is a classic vi-modal-confusion bug).

## Considered Options

1. **`DiffSource::Pristine` variant carries the focused file set** -- rejected, locks the workflow to pristine mode when commit-range and PR reviews want the same focus surface.
2. **Filter the rendered line stream at the renderer** -- rejected, cursor math and rendering desynchronize (cursor lands on lines that aren't rendered, `next_hunk` walks past non-rendered hunks).
3. **Mode flag + render-aware height helper + file-walking cursor** -- chosen.

## Decision Outcome

Chosen: option 3.

The toggle is `:focus` (alias `:f`, leader-prefixed `<leader>f`). `--all-files` defaults to single-file view since whole-repo continuous scroll is the worse default for that surface; every other mode opens in the existing continuous view.

The per-file `═══ filename ═══` separator and the global `═══ Review Comments ═══` banner are dropped in single-file view -- both are redundant with only one file on screen, and both eat horizontal space when long lines force the user to scroll. A `↓ <next-filename>` hint replaces the inter-file blank so the user can see what's on the other side of `j`. Reviewed files render the body under a dimmed `Marked reviewed -- r to re-open` banner instead of collapsing to a header.

Cursor and scroll math route through `effective_file_height(idx, file)`. Multi-file: same as `file_render_height`. Single-file: 0 for non-current files; current file gets body + banner (no header, no reviewed-collapse short-circuit). `total_lines`, `calculate_file_scroll_offset`, `next_hunk`, and `prev_hunk` all consume the helper, so cursor / scroll / hunk-walk semantics agree with what the renderer draws.

`j` past the last line of a file does not immediately walk to the next file -- it arms `primed_walk_next` and parks the cursor on max. The consume gate also requires `down_released_since_arm` (set by a Down/`j` Release event), so held-j auto-repeats fire Press, Repeat, Repeat, ..., Release and never satisfy the gate. A deliberate release + second press walks. The same kitty `REPORT_EVENT_TYPES` enhancement that distinguishes Press from Repeat is pushed at terminal init alongside `DISAMBIGUATE_ESCAPE_CODES`; the event filter widens to accept both kinds so auto-repeat continues to drive cursor movement within a file. On terminals that don't support the enhancement, `supports_keyboard_enhancement` is false and the release gate is bypassed (two press events walk, since Release is never emitted). Symmetric for `k` / Up at the top of a file via `primed_walk_prev` + `up_released_since_arm`. `]` and `[` walk hunks within the current file, and once exhausted cross into the next / previous file's first / last hunk so a single keystream can step the codebase hunk-by-hunk without needing to know file boundaries.

File-list `j`/`k` auto-follows in single-file view: the highlighted file becomes the visible one without pressing Enter. Mouse-wheel on the file list stays as viewport scroll (no auto-follow) because wheel is a "browse" gesture, not a "navigate" gesture.

## Consequences

- [+] Single-file view is a first-class mode that composes with every existing surface: comments, clipboard, persistence, themes, file-tree expansion, search.
- [+] Holding `j` walks the codebase file-by-file. The previous workaround -- jump-to-file from the file list, repeat -- collapses into a single keystroke.
- [+] The `effective_file_height` helper gives every geometry-aware code path one place to ask "how tall is this file in the current view?". Adding future modes (e.g. focus on a directory subtree) is a small extension to one helper, not a per-call-site decision.
- [-] Search and `n`/`N` cursor placement (not touched in this PR) likely have the same cumulative-offset shape as `next_hunk` did and may need the same audit.
- [-] Sticky mode adds a dimension of "why is `j` behaving differently?" confusion. Mitigated by the `FOCUS` status chip.

Entering Comment mode snaps `diff_state.scroll_x = 0` so the inline input box always lands inside the viewport. Without the snap, opening a comment on a line scrolled past the right edge leaves the input box off-screen and the user types blind until pressing Enter.

## Future Considerations

- Audit search and `n`/`N` cursor placement against `effective_file_height`; today they iterate cumulative offsets the same way `next_hunk` did before this change.
- Pristine `--all-files` over a very large repo loads every tracked file at startup; the continuous scroll is gone but the initial load isn't paginated. A lazy file-tree load (only build the diff for the visible file plus a small window) would scale further.
- Consider per-session toggle persistence so a user who toggled `:focus` once does not have to re-toggle every launch.

[FEAT-0012]: FEAT-0012 Arrow Navigation.md
