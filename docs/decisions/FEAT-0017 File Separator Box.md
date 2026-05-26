---
title: File Separator Box
description: Delta-style box separator between files in multi-file view, with cursor skip over decoration lines
type: adr
status: proposed
created: 2026-05-26
---

# FEAT-0017 File Separator Box

## Context

The single `═══ filename [M] ═══` rule separating files in multi-file view is visually heavy and hard to distinguish from diff content at a glance. The cursor can land on the header line, which has no actionable behavior (no commenting, no reviewing). Scrolling through a multi-file diff requires extra keypresses to traverse decoration.

## Decision

File headers render as a three-line box that spans the panel from edge to edge. The
indicator gutter is dropped on these rows since the cursor never lands on them:

```
  42 │  }

╔════════════════════════════════════════════════╗
║ ✓ src/app.rs [M]                               ║
╚════════════════════════════════════════════════╝
   1 │  use std::path::Path;
``` The trailing `Spacing` annotation from the previous file provides a blank line above.

Three new annotation types support the box: `FileHeaderBorder` for top/bottom borders, the existing `FileHeader` for the filename row, and `Spacing` for the blank line. Cursor movement (`j`/`k`, arrows) skips all decoration lines (`Spacing`, `FileHeader`, `FileHeaderBorder`) and lands on the next content line.

The `paint_file_header_fill` overlay function detects the line type by its leading characters (`╔═` → fill `═` with `╗` corner, `╚═` → fill `═` with `╝` corner, `║ ` → fill ` ` with `║` closing bar) and extends the box to viewport width.

## Consequences

- [+] File boundaries are visually distinct from diff content without shouting.
- [+] One keypress crosses the entire separator, matching the mental model of "next file."
- [-] Three annotation lines per file header instead of one; affects `total_lines` and scroll offset calculations proportionally.
