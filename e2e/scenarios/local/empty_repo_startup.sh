#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# A repository with no commits and no changes has nothing to review: tuicr exits 1 with "No changes to review" (no empty TUI).
# Adding a file makes the same repository reviewable: the selector then offers the unstaged change.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; rm -rf $d/repo; git init -q -b main $d/repo
cd $d/repo
sc_capture $n --no-update-check
[ "$SC_RC" = 1 ] || { echo "expected exit 1, got $SC_RC"; echo "$SC_OUT"; exit 1; }
grep -q 'No changes to review' <<<"$SC_OUT" || { echo "no empty-state message:"; echo "$SC_OUT"; exit 1; }
printf 'first file\n' > $d/repo/new.txt
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n Enter
e2e_wait_for $n 'new\.txt'; e2e_expect $n 'first file'
sc_quit $n
