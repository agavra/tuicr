---
title: Mouse Horizontal Scroll Routing
description: Trackpad ScrollLeft/Right dispatches directly to the diff viewport, bypassing the keyboard focus-slide
type: adr
status: proposed
created: 2026-05-21
---

# FEAT-0013 Mouse Horizontal Scroll Routing

## Context and Problem Statement

Keyboard arrow keys / `h` / `l` are wired to `Action::ScrollLeft` and `Action::ScrollRight`, which carry two responsibilities: scroll the diff viewport horizontally, and shift focus between panes at the edges (see [FEAT-0012][FEAT-0012]). Routing mouse and trackpad horizontal events (`MouseEventKind::ScrollLeft` / `ScrollRight`) through the same `Action` path is the obvious default, but it breaks under trackpad input.

A two-finger swipe over the diff at `scroll_x = 0` shifts focus to the file list. A swipe over the file list dismisses the panel. A swipe during an active `VisualSelect` aborts the selection. None of these are gestures the user means to perform: the trackpad is a scroll input, not a panel-management input.

## Decision Drivers

- Keyboard arrow keys are intentional. Each press is a decision the user made.
- Trackpad horizontal scroll is incidental. A two-finger horizontal swipe happens whenever the user moves the cursor across the trackpad surface, with no panel intent.
- Long lines in source code still need to be scrollable horizontally. Removing mouse `ScrollLeft`/`Right` entirely would make wide files unreadable in unified diff view.

## Considered Options

### 1. Single `Action` dispatch for keyboard and mouse

Route mouse `ScrollLeft`/`Right` through `Action::ScrollLeft`/`Right` like the arrow keys.

Rejected: documented above. The shared path means every trackpad swipe triggers panel-management side effects.

### 2. Drop mouse `ScrollLeft`/`Right` entirely

Make horizontal mouse events no-ops. Users scroll wide files with `h`/`l` from the keyboard.

Rejected: removes a working feature. Trackpad horizontal scroll is a real input modality.

### 3. Bypass the `Action` layer for mouse horizontal scroll

Mouse `ScrollLeft`/`Right` dispatches directly to `app.scroll_left` / `app.scroll_right`. The keyboard `Action::ScrollLeft`/`Right` path that triggers focus-slide and hide-reveal is bypassed entirely.

## Decision Outcome

Chosen: option 3.

The mouse event handler in `src/handler.rs` matches `MouseEventKind::ScrollLeft | ScrollRight`, checks that the pointer is over the diff panel, checks that the input mode is one where horizontal scroll makes sense (Normal or VisualSelect), and calls `app.scroll_left` / `app.scroll_right` directly. The keyboard arrow keys continue to produce `Action::ScrollLeft` / `ScrollRight`, which thread through `handle_diff_action` / `handle_file_list_action` and trigger the gestures from FEAT-0012.

Mouse events over the file list are dropped. The file list itself rarely needs horizontal scrolling (filenames almost always fit), and any panel-management gesture wired to file-list mouse swipes would suffer the same incidental-trigger problem as the diff side.

## Consequences

- [+] Trackpad horizontal scroll works for long lines without any focus-slide or hide side effects.
- [+] `VisualSelect` survives a horizontal swipe.
- [+] Keyboard focus-slide gestures stay discoverable and intentional. They fire on key presses, not on every trackpad gesture.
- [-] Mouse and keyboard now diverge on the same logical event class. A developer adding a future scroll-related behavior needs to remember to wire both paths (or consciously decide one is keyboard-only).
- [-] File-list horizontal scroll via mouse is permanently gone. Filenames truncated at the panel edge cannot be revealed by mouse; the keyboard `h` in file list still works. No user data yet suggests this is a real regression.

## Future Considerations

- If a future input modality (touchscreen?) is added, decide its routing at that point. Default should be "intentional gestures share the keyboard `Action` path; incidental gestures bypass."
- If file-list horizontal scroll matters enough to add back, route mouse swipes over the file list to `app.file_list_state.scroll_left` / `scroll_right` directly, mirroring the diff path.

[FEAT-0012]: FEAT-0012 Arrow Navigation.md
