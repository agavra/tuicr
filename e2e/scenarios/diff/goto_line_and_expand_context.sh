#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `:N` (new side), `:oN` (old side) and `{N}G` put the cursor on that source line even inside collapsed context, expanding
# only as far as needed; `Enter` on an "expand" row reveals the hidden lines and `Enter` on revealed context folds them back.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; r=$d/repo; rm -rf $r; mkdir -p $r; git init -q -b main $r
seq 1 100 | sed 's/^/row /' > $r/f.txt; echo x > $r/g.txt; _fx_git $r add -A; _fx_git $r commit -qm base
# three lines replace "row 5", so later rows are numbered +2 on the new side
python3 - $r/f.txt <<'PY'
import sys
p = sys.argv[1]; s = open(p).read().replace("row 5\n", "ins a\nins b\nins c\n").replace("row 70\n", "row 70 CHANGED\n"); open(p, "w").write(s)
PY
E2E_CWD=$r e2e_start $n "$TUICR_BIN" --no-update-check -w
e2e_wait_for $n 'row 70 CHANGED'; e2e_keys $n Escape
e2e_reject $n 'row 28'                                                 # hidden to begin with
sc_cmd $n 30;  e2e_wait_for $n '▶ +30 +row 28'                           # new-side line 30 is "row 28"
sc_cmd $n o30; e2e_wait_for $n '▶ +32 +row 30'                           # old-side line 30 is "row 30"
e2e_keys $n 5 0 G; e2e_wait_for $n '▶ +50 +row 48'
# fresh view: Enter on an expand row reveals its lines, Enter on revealed context folds them again
sc_cmd $n e; e2e_wait_gone $n 'row 48'
e2e_keys $n g; sc_cursor_on $n 'expand \(20 lines\)'
e2e_keys $n Enter; e2e_wait_for $n 'row 28'
e2e_keys $n Enter; e2e_wait_gone $n 'row 28'
sc_quit $n
