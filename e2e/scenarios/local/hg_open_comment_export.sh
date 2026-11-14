#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Mercurial journey end to end: open the uncommitted changes, comment on an added line, export with --stdout. The markdown
# anchors the comment to the right file:line. The fixture changes two files of one status class (modified) on purpose:
# mixed status classes are covered by local/hg_hunks_under_own_files. Needs real Mercurial (SKIP without it, FAIL with E2E_REQUIRE_HG=1).
e2e_use_hg
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; rm -rf $d/repo; mkdir -p $d/repo   # only modified files: see the note in the header
( cd $d/repo && hg init -q . && printf 'a_old\n' > a.txt && printf 'z_old\n' > z.txt && hg add -q . && hg commit -q -u e2e -m base \
    && printf 'a_new\n' > a.txt && printf 'z_new\n' > z.txt )
out=$d/stdout.txt; err=$d/stderr.txt; rm -f $out $err
E2E_CWD=$d/repo e2e_start $n sh -c "exec '$TUICR_BIN' --no-update-check --stdout >'$out' 2>'$err'"
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n Enter
e2e_wait_for $n 'Files · 0/2'; e2e_keys $n Escape; sc_cursor_on $n '1 ▌ z_new'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'hg exported note'; e2e_keys $n Enter; e2e_wait_for $n 'hg exported note'
sc_cmd $n clip; e2e_wait_exit $n
grep -q '^I reviewed your code' $out || { echo "export lacks the intro"; cat $out; exit 1; }
grep -qx '1\. `z\.txt:1` - hg exported note' $out || { echo "export lacks the anchored comment"; cat $out; exit 1; }
grep -q 'tuicr-summary: reviewed 0/2 files, 1 comment added' $err || { echo "stderr lacks the summary"; cat $err; exit 1; }
