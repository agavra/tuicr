---
title: File List Visibility
description: Tab reveals the file list when the focus cycle lands on it, and Esc hides it again
type: adr
status: proposed
created: 2026-05-24
---

# FEAT-0016 File List Visibility

## Context

The file list can be hidden (`:e` toggle, `--no-file-list`, narrow terminal). Once hidden, two awkward states emerge:

- Tab / Shift+Tab still cycle focus through every panel, including the hidden file list. Focus lands on a panel the user can't see, with no visible cursor and no way to act on it.
- The diff is the panel a reviewer spends most time in, but there's no fast keystroke to dismiss the file list once it's served its purpose. `:e` works but requires command-mode entry.

## Decision

Tab / Shift+Tab unhide the file list whenever the focus cycle lands on it. The reveal is automatic and unconditional -- a focused-but-hidden panel is invisible state, so the panel surfaces whenever focus enters it.

Esc in any normal-mode panel hides the file list and shifts focus to the diff. This makes Esc the universal "back to the diff, quiet the chrome" keystroke. `:e` and the leader binding (`<leader>e`) still toggle visibility for users who prefer explicit commands.

## Consequences

- [+] Tab can no longer leave focus on an invisible panel.
- [+] Esc gives reviewers a one-key path back to a clean diff view.
- [-] Esc in normal mode used to only shift focus to the diff. Hiding the file list is a behavior change for users who hit Esc as a focus-only gesture; mitigated by the focus shift still happening and `:e` / `<leader>e` bringing the file list back.
