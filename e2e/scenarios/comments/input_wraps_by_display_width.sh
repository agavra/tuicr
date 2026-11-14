#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #906: comment input wraps by display width. VS16 emoji (U+2764 U+FE0F) count 1 per char in the baseline's
# wrap_segments, so a row holds twice the box width and spills over the right border; the wrapped rows must stay inside the box.
n=$(e2e_name)
sc_diff $n; e2e_keys $n j j; e2e_keys $n c; e2e_wait_for $n 'Add L1' 3
e2e_keys $n -l "$(python3 -c 'print("❤️"*60)')"; e2e_settle $n 0.4
rows=$(e2e_screen $n | grep '❤')
[ -n "$rows" ] || { echo "no emoji rows"; exit 1; }
# tmux's own VS16 width handling garbles the *last* wrapped row, so judge the first row only: it must close at the border.
first=$(head -1 <<<"$rows")
grep -q '││$' <<<"$first" || { echo "first wrapped emoji row spills past the comment box border"; exit 1; }
[ "$(wc -l <<<"$rows")" -ge 2 ] || { echo "60 emoji did not wrap inside the box (single overlong row)"; exit 1; }
e2e_keys $n C-c; e2e_settle $n
sc_quit $n
