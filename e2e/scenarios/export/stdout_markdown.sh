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
# (the slug lists the range newest..oldest as the selector shows it; the commits line reads oldest first)
# the whole document, not scattered substrings: session heading, intro, the reviewed scope (newest commit first), the
# "Local tuicr Comments" section with ONE numbered item anchored at file:line, and the machine-readable stderr markers
new=$(git -C $d/repo rev-parse --short=7 HEAD); old=$(git -C $d/repo rev-parse --short=7 HEAD~1)   # HEAD~2..HEAD = these two commits
want=$(printf '## Session: repo@main/commits/%s..%s\n\nI reviewed your code and have the following comments. Please address them.\n\nReviewing commits: %s, %s\n\n## Local tuicr Comments\n\n1. `README.md:4` - export me please' \
  $new $old $old $new)
[ "$(cat $out)" = "$want" ] || { echo "exported markdown differs"; echo "--- want"; echo "$want"; echo "--- got"; cat $out; exit 1; }
grep -qx "tuicr-session: repo@main/commits/$new\.\.$old" $err || { echo "stderr lacks tuicr-session"; cat $err; exit 1; }
grep -qx 'tuicr-summary: reviewed 0/8 files, 1 comment added' $err || { echo "stderr lacks the summary"; cat $err; exit 1; }
