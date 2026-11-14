#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `:N` into a line hidden inside a collapsed (reviewed) file must not expand the gap behind the
# user's back. Baseline expanded context rows 8..30 of the gap; once the file is un-reviewed they show up.
n=$(e2e_name); d=$(e2e_dir $n); rm -rf "$d"; mkdir -p "$d/repo"; r="$d/repo"
git init -q -b main "$r"; seq 1 100 | sed 's/^/row /' > "$r/f.txt"; echo x > "$r/g.txt"
_fx_git "$r" add -A; _fx_git "$r" commit -qm base
python3 - "$r/f.txt" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read().replace("row 5\n", "row 5 CHANGED\n").replace("row 60\n", "row 60 CHANGED\n")
open(p, "w").write(s)
PY
echo y > "$r/g.txt"
E2E_CWD="$r" e2e_start $n "$TUICR_BIN" --no-update-check -w
e2e_wait_for $n 'f.txt'; e2e_keys $n Escape
e2e_keys $n r; e2e_wait_for $n 'Files · 1/2'          # f.txt reviewed => collapsed
e2e_keys $n ":30" Enter; e2e_settle $n
e2e_keys $n r; e2e_wait_for $n 'Files · 0/2'          # un-review, f.txt is rendered again
e2e_wait_for $n 'row 5 CHANGED'
e2e_settle $n
scr=$(e2e_screen $n)
if grep -Eq 'row (20|30)( |$)' <<<"$scr"; then echo "gap was expanded while the file was collapsed"; echo "$scr"; exit 1; fi
grep -Eq 'lines hidden' <<<"$scr" || { echo "expected a still-collapsed gap"; e2e_screen $n; exit 1; }
sc_quit $n
