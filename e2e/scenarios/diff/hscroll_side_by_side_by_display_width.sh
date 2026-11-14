#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #888: side-by-side: horizontal scroll (`l`, 4 columns) pans by display width and pads a wide glyph that
# the pan cuts in half, so the divider stays in place. Row 5 is `a` + CJK: skipping 4 columns cuts the 2nd wide glyph.
# Baseline pans by characters: row 5 jumps to a different place (no pad cell).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_wide_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~1..HEAD
e2e_wait_for $n 'Files · 0/4'; e2e_keys $n Escape
sc_cmd $n 'set wrap!'; sc_cmd $n diff
e2e_wait_for $n '5 ▌a日本語のテキスト' 
e2e_keys $n l
e2e_wait_for $n '5 ▌ 語のテキスト' || { echo "wide glyph cut by the pan was not padded (panned by characters, not cells)"; e2e_screen $n; exit 1; }
# every pane row keeps the divider: each diff row of the hunk still holds both panes' separators
e2e_screen $n | grep -E '^│.*││ +[0-9]+ .*│ +[0-9]+ ' | wc -l | grep -qv '^0$' || { echo "divider rows missing"; exit 1; }
sc_quit $n
