---
title: Arrow Navigation
description: Left/right arrow keys slide focus between the file list and the diff at the scroll boundary
type: adr
status: proposed
created: 2026-05-21
---

# FEAT-0012 Arrow Navigation

## Context

`<leader>h` / `<leader>l` and `Tab` switch focus between the file list and the diff. Two keystrokes for what is conceptually a single direction-of-attention gesture, and neither corresponds to the natural "I've scrolled to the edge, give me the next panel" motion that one-handed home-row navigation rewards.

## Decision

`h` / `l` / Left / Right scroll the focused panel horizontally while there's hidden content to scroll, then fall through to a focus slide at the scroll boundary. Gated by `arrow_tree_navigation` (default `false`) so vim users keep `h` / `l` as pure horizontal scroll. The flag also governs the gitui-style tree expansion in the file list (see [FEAT-0014][FEAT-0014]).

**In the diff:**

- `h` while `scroll_x > 0` scrolls left.
- `h` at `scroll_x == 0` slides focus to the file list, revealing it if hidden.
- `l` scrolls right.

**In the file list:**

- `l` while `scroll_x < max_scroll_x` scrolls right.
- `l` at `scroll_x == max_scroll_x` on a file slides focus to the diff. On a folder, the tree-nav rules from [FEAT-0014][FEAT-0014] apply instead.
- `h` scrolls left, then falls through to tree-nav at the leftmost column.

Mouse horizontal scroll bypasses this layer entirely; see [FEAT-0013][FEAT-0013].

## Consequences

- [+] One-keystroke pane switching at the natural scroll boundary for users who opt in.
- [+] Hidden file list reveals itself the moment `h` in the diff tries to slide into it, so a hidden panel is never an obstacle.
- [-] The slide trigger conditions on the scroll boundary, which is internal state the user has to discover. On a wide file-list panel everything fits, so the very first `l` press already slides.

[FEAT-0013]: FEAT-0013 Mouse Horizontal Scroll Routing.md
[FEAT-0014]: FEAT-0014 Folder Expansion.md
