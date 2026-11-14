#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# a `:e` reload of PR #1 that is still in flight when the user opens PR #2 must be dropped.
# Before, its late result was applied to the PR now on screen (PR #2 showed PR #1's files).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; echo rich > $d/variant; rm -rf $d/hold; mkdir -p $d/hold
E2E_ENV="E2E_VARIANT_FILE=$d/variant E2E_HOLD_DIR=$d/hold" sc_pr_open $n
e2e_wait_for $n 'PR #1'
# start a reload and hold its data fetch (files/commits/diff of PR #1)
touch $d/hold/gate; sc_cmd $n e
# barrier: all three data calls (files, commits, diff) of the reload are parked on the forge
e2e_wait_forge_count '"note": "HELD' 3 10 || { echo "reload data calls never reached the forge hold"; exit 1; }
rm $d/hold/gate; echo pr2 > $d/variant
# open PR #2 from the selector while the reload is parked
sc_cmd $n prs; e2e_wait_for $n 'Second PR other change' 10
e2e_keys $n j; e2e_settle $n; e2e_keys $n Enter
e2e_wait_for $n 'PR #2|octo/demo#2' 15; e2e_wait_for $n 'other\.rs' 10
# now let PR #1's reload finish
touch $d/hold/release
# barrier: the three held data calls of PR #1's reload were served; then the UI absorbs (and must drop) the result
e2e_wait_forge_count '"note": "RELEASED' 3 10 || { echo "held reload calls were never released"; exit 1; }
e2e_settle $n 0.4
scr=$(e2e_screen $n)
grep -q 'octo/demo#2' <<<"$scr" || { echo "PR #2 is no longer on screen"; echo "$scr"; exit 1; }
! grep -q 'greet\.rs' <<<"$scr" || { echo "PR #1's late reload result replaced PR #2's files"; echo "$scr"; exit 1; }
grep -q 'other\.rs' <<<"$scr" || { echo "PR #2's file list lost"; e2e_screen $n; exit 1; }
sc_quit $n
