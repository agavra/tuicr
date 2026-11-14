#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Marking ONE hunk reviewed (R) collapses just that hunk to a "✓ @@" row; after :w, quit and restart the same hunk (and only it)
# is still collapsed; R on it again restores its lines and the saved session forgets the mark.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; rm -rf $d/repo; git init -q -b main $d/repo
seq 1 200 | sed 's/^/row /' > $d/repo/many.txt; _fx_git $d/repo add -A; _fx_git $d/repo commit -qm base
python3 - $d/repo/many.txt <<'PY'
import sys
p = sys.argv[1]; L = open(p).read().split('\n')
for i, w in ((10, 'ten'), (100, 'hundred'), (190, 'ninety')): L[i - 1] = 'row %s CHANGED' % w
open(p, 'w').write('\n'.join(L))
PY
open_diff() {
  E2E_CWD=$d/repo e2e_start_keep $n "$TUICR_BIN" --no-update-check
  e2e_wait_for $n 'Unstaged changes'; e2e_keys $n Enter; e2e_wait_for $n 'many\.txt'
  e2e_wait_for $n '@@ -187,7 \+187,7 @@'
}
open_diff
e2e_keys $n ']' ']'; e2e_wait_for $n '▶ +@@ -97,7 \+97,7 @@'           # the middle hunk
e2e_expect $n 'row hundred CHANGED'
e2e_keys $n R; e2e_wait_gone $n 'row hundred CHANGED'; e2e_expect $n '✓ @@ -97,7 \+97,7 @@'
e2e_expect $n 'row ten CHANGED'; e2e_expect $n 'row ninety CHANGED'      # the other hunks are untouched
e2e_expect $n 'Files · 0/1'                                              # a hunk mark is not a file mark
sc_save $n; sc_cmd $n 'q!'; e2e_wait_exit $n
sc_session_json $n | python3 -c '
import json, sys
j = json.load(sys.stdin); fr = j["files"]["many.txt"]
assert len(fr["reviewed_hunks"]) == 1 and not fr["reviewed"], fr' || { echo "saved session lacks exactly one hunk mark"; sc_session_json $n | head -c 1500; exit 1; }
# restart: the same hunk is still reviewed, the others are not
open_diff
e2e_wait_for $n '✓ @@ -97,7 \+97,7 @@'; e2e_wait_gone $n 'row hundred CHANGED'
e2e_expect $n 'row ten CHANGED'; e2e_expect $n 'row ninety CHANGED'; e2e_reject $n '✓ @@ -7,7|✓ @@ -187,7'
# toggle it off again: lines are back, and a saved session forgets the mark
e2e_keys $n ']' ']'; e2e_wait_for $n '▶ +✓ @@ -97,7 \+97,7 @@'
e2e_keys $n R; e2e_wait_for $n 'row hundred CHANGED'; e2e_wait_gone $n '✓ @@'
sc_save $n
sc_session_json $n | python3 -c '
import json, sys
j = json.load(sys.stdin); assert j["files"]["many.txt"]["reviewed_hunks"] == [], j["files"]["many.txt"]' \
  || { echo "hunk mark survived the toggle-off"; sc_session_json $n | head -c 1500; exit 1; }
sc_quit $n
