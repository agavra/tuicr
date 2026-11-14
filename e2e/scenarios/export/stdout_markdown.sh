#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# --stdout: :clip then quit prints the review markdown to stdout (captured in a file); stderr markers.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
out=$d/stdout.txt; err=$d/stderr.txt; rm -f $out $err
E2E_CWD=$d/repo e2e_start $n sh -c "exec '$TUICR_BIN' --no-update-check --stdout -r HEAD~2..HEAD >'$out' 2>'$err'"
e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'export me please'; e2e_keys $n Enter; e2e_wait_for $n 'export me please'
sc_cmd $n clip
e2e_wait_exit $n   # with --stdout, :clip prints the export and ends the session
grep -q 'export me please' $out || { echo "stdout lacks the comment"; echo "--- stdout"; cat $out; echo "--- stderr"; cat $err; exit 1; }
grep -q 'README.md' $out || { echo "stdout lacks file path"; cat $out; exit 1; }
grep -q 'I reviewed your code' $out || { echo "stdout lacks the intro"; cat $out; exit 1; }
grep -q 'tuicr-summary:' $err || { echo "stderr lacks tuicr-summary marker"; cat $err; exit 1; }
