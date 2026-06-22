---
title: Folder Expansion
description: Opt-in gitui-style folder expand/collapse on h/l in the file list, the diff follows the file-list cursor, and the sticky prefix keeps badges visible during horizontal scroll
type: adr
status: proposed
created: 2026-05-23
---

# FEAT-0014 Folder Expansion

## Context

Default `h` / `l` are vim's character-navigation keys; in tuicr that maps to horizontal scroll of the focused panel. Some users would rather have those keys drive the file tree the way gitui does -- expand / collapse / descend / ascend. The two intents collide on the same keys, so the gitui behaviour is gated behind a config flag and off by default.

The same audience also expects browse-as-you-go: moving the file-list cursor onto a different file scrolls the diff to that file's header, the way gitui and lazygit preview files. Without it the user has to press Enter to commit each selection, which is awkward when you can already see which file you're on.

Long filenames in deep paths spill past the panel's right edge. The whole row including its status badge slides off under horizontal scroll, leaving the user with no idea which file they're on. The fix is to anchor the badge and indent at the left edge and slide only the name portion.

## Decision

When `arrow_tree_navigation = true`, `h` / `l` in the file list fall through to gitui-style tree nav at the horizontal scroll boundary:

- `l` on a collapsed folder expands it; on an expanded folder descends to the first child; on a file jumps to the next folder below.
- `h` on an expanded folder collapses it; otherwise ascends to the parent. At the top level it jumps to the previous folder above.

Default is `false`: `h` / `l` only scroll horizontally, vim-style. Folders are still expandable with `Enter` / `Space` regardless of the flag.

With the flag on, moving the file-list cursor (`j` / `k`, arrows, or the tree-nav keys above) to a file also scrolls the diff to that file's header. Focus stays on the file list so the user can keep browsing. Folders are a no-op so arrowing past collapsed entries doesn't churn the diff viewport. Enter still commits a selection and shifts focus, matching the existing pattern.

Horizontal scroll keeps the sticky prefix (indent, expand icon, checkbox, status badge) anchored at the left edge; only the filename portion slides. Scroll is capped so at least one column of the longest name stays visible. Leaving the file list resets `scroll_x` to 0 so long names re-enter from the start next time focus returns.

## Consequences

- [+] Vim-pure default. Users who want tree nav opt in.
- [+] Browse-as-you-go matches gitui / lazygit muscle memory.
- [+] Long filenames stay identifiable mid-scroll because the badge never disappears.
- [-] The opt-in is invisible without reading the config docs.
- [-] The diff scrolls every time the file-list cursor lands on a different file, which can be jarring if the user is mid-read. Mitigated by keeping the opt-in off by default.
