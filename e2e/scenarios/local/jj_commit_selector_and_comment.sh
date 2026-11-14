#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# jj backend: the selector lists jj commits (working copy first); Enter reviews the working-copy change with its files
# and hunks; a comment saves under the jj session. Needs real jj (SKIP without it, FAIL with E2E_REQUIRE_JJ=1).
e2e_use_jj
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_jj_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'wip'; e2e_expect $n 'second'; e2e_expect $n 'base'; e2e_expect $n 'jj:'
e2e_keys $n Enter
e2e_wait_for $n 'Files · 0/3'; e2e_expect $n 'M a\.txt'; e2e_expect $n 'A d\.txt'; e2e_expect $n 'a wip'; e2e_expect $n 'd wip'
e2e_reject $n 'c one'                                                    # c.txt belongs to "second", not to the working copy
e2e_keys $n Escape; sc_cursor_on $n 'a wip'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'jj note'; e2e_keys $n Enter; e2e_wait_for $n 'jj note'
sc_save $n; sc_session_has $n 'jj note' || { echo "jj session not saved"; exit 1; }
sc_quit $n
