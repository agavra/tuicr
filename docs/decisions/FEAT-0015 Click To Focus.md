---
title: Click to Focus
description: Clicking anywhere on a panel (border or empty space included) sets focus to that panel
type: adr
status: proposed
created: 2026-05-23
---

# FEAT-0015 Click to Focus

## Decision

Mouse Down on either panel's outer area sets `focused_panel` to that panel. Outer area includes the border and any empty space below the last row, not just the inner content rows. Inner-area clicks still position the cursor or start a visual selection.

## Consequences

- [+] Clicking on a panel's border or empty space no longer feels broken -- focus moves where the user pointed.
- [+] Inner-area click behavior is unchanged; no regression for cursor positioning or visual selection start.
