#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# jj backend: `-r @-` reviews the working copy's parent (adds, edits and a deletion with their own hunks); a Space-selected range in the
# selector reviews both commits cumulatively. Needs real jj.
e2e_use_jj
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_jj_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r '@-'
e2e_wait_for $n 'Files · 0/4'; e2e_expect $n 'M a\.txt'; e2e_expect $n 'D b\.txt'; e2e_expect $n 'A c\.txt'
e2e_expect $n 'c one'; e2e_expect $n 'a two'; e2e_reject $n 'a wip|d wip'   # nothing of the working-copy change
sc_quit $n
m=${n}_range; E2E_CWD=$d/repo e2e_start $m "$TUICR_BIN" --no-update-check
e2e_wait_for $m 'wip'; e2e_keys $m Space Space Enter                      # rows: wip, second -> both
e2e_wait_for $m 'Files · 0/4'; e2e_expect $m 'A c\.txt'; e2e_expect $m 'A d\.txt'; e2e_expect $m 'D b\.txt'
e2e_expect $m 'a wip'; e2e_expect $m 'a two'
sc_quit $m
