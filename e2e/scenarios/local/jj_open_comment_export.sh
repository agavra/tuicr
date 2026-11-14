#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# jj journey end to end: open the working-copy change, comment on a line, export with --stdout. The markdown carries the jj
# jj scope (the working-copy change) and the comment anchored at its file:line. Needs real jj (SKIP without it, FAIL with E2E_REQUIRE_JJ=1).
e2e_use_jj
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_jj_repo $d/repo
out=$d/stdout.txt; err=$d/stderr.txt; rm -f $out $err
E2E_CWD=$d/repo e2e_start $n sh -c "exec '$TUICR_BIN' --no-update-check --stdout >'$out' 2>'$err'"
e2e_wait_for $n 'wip'; e2e_keys $n Enter
e2e_wait_for $n 'Files · 0/3'; e2e_keys $n Escape; sc_cursor_on $n '3 ▌ a wip'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'jj exported note'; e2e_keys $n Enter; e2e_wait_for $n 'jj exported note'
sc_cmd $n clip; e2e_wait_exit $n
grep -q '^I reviewed your code' $out || { echo "export lacks the intro"; cat $out; exit 1; }
grep -Eqx '1\. `a\.txt:3` \(commit [0-9a-f]+\) - jj exported note' $out || { echo "export lacks the anchored comment"; cat $out; exit 1; }
grep -q 'tuicr-summary: reviewed 0/3 files, 1 comment added' $err || { echo "stderr lacks the summary"; cat $err; exit 1; }
