#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# a visual (range) selection that crosses a file boundary must be rejected, not turned into a
# comment on lines of whichever file the numbers happen to fit. Baseline opened the comment box for the cross-file range.
n=$(e2e_name); d=$(e2e_dir $n); rm -rf "$d"; mkdir -p "$d/repo"; r="$d/repo"
git init -q -b main "$r"; printf 'a1\na2\na3\n' > "$r/a.txt"; printf 'b1\nb2\nb3\n' > "$r/b.txt"
_fx_git "$r" add -A; _fx_git "$r" commit -qm base
printf 'a1\na2 changed\na3\n' > "$r/a.txt"; printf 'b1\nb2 changed\nb3\n' > "$r/b.txt"
E2E_CWD="$r" e2e_start $n "$TUICR_BIN" --no-update-check -w
e2e_wait_for $n 'a.txt'; e2e_keys $n Escape
sc_cursor_on $n '2 ▌ a2 changed'
e2e_keys $n v j j j j j j j; e2e_settle $n
e2e_keys $n c; e2e_wait_for $n 'Invalid selection|COMMENT'
e2e_screen $n > $d/after.txt
if grep -q 'COMMENT' $d/after.txt; then echo "comment box opened for a cross-file selection"; cat $d/after.txt; exit 1; fi
grep -q 'Invalid selection' $d/after.txt || { echo "no 'Invalid selection' warning"; cat $d/after.txt; exit 1; }
# control: a single-file range still opens the comment box
e2e_keys $n v j; e2e_keys $n c; e2e_wait_for $n COMMENT
e2e_keys $n Escape
sc_quit $n
