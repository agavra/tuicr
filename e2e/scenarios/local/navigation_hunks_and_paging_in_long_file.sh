#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# One file with three distant hunks (lines 10, 100, 190 of 200): `]`/`[` step hunk by hunk inside the file and stop at both ends,
# Ctrl-d / Ctrl-u page the cursor down and back up to known rows (a no-op paging implementation cannot pass).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; rm -rf $d/repo; git init -q -b main $d/repo
seq 1 200 | sed 's/^/row /' > $d/repo/many.txt; _fx_git $d/repo add -A; _fx_git $d/repo commit -qm base
python3 - $d/repo/many.txt <<'PY'
import sys
p = sys.argv[1]; L = open(p).read().split('\n')
for i, w in ((10, 'ten'), (100, 'hundred'), (190, 'ninety')): L[i - 1] = 'row %s CHANGED' % w
open(p, 'w').write('\n'.join(L))
PY
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n Enter; e2e_wait_for $n 'many\.txt'
e2e_wait_for $n '@@ -187,7 \+187,7 @@'                                      # all three hunks are on screen at this size
at() { e2e_wait_for $n "▶ +$1"; }                                          # the cursor row shows this text
at '═══ many\.txt'
# paging: half a page down lands on row 101, the next one on the last expand row, Ctrl-u comes back up
e2e_keys $n C-d; at '101 +row 101'
e2e_keys $n C-d; at '\.\.\. ↓ expand \(7 lines\)'
e2e_keys $n C-u; at '97 +row 97'
# hunks: forward through all three, no wrap past the last, then back
e2e_keys $n g; at '\.\.\. ↑ expand \(6 lines\)'   # top of the file's content
e2e_keys $n ']'; at '@@ -7,7 \+7,7 @@'
e2e_keys $n ']'; at '@@ -97,7 \+97,7 @@'
e2e_keys $n ']'; at '@@ -187,7 \+187,7 @@'
e2e_keys $n ']'; e2e_settle $n; at '@@ -187,7 \+187,7 @@'                  # last hunk: stays
e2e_keys $n '['; at '@@ -97,7 \+97,7 @@'
e2e_keys $n '['; at '@@ -7,7 \+7,7 @@'
sc_quit $n
