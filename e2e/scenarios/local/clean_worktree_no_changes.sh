#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# A committed, clean repository: `-w` (working tree) has nothing to review and exits 1 with "No changes to review", while the
# default startup still opens the selector listing the commit history. Dirtying the tree makes `-w` work.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
git -C $d/repo add -A; git -C $d/repo -c user.name=E2E -c user.email=e2e@example.invalid commit -qm "settle the work tree"
cd $d/repo
sc_capture $n --no-update-check -w
[ "$SC_RC" = 1 ] || { echo "expected exit 1, got $SC_RC"; echo "$SC_OUT"; exit 1; }
grep -q 'No changes to review' <<<"$SC_OUT" || { echo "no empty-state message:"; echo "$SC_OUT"; exit 1; }
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'settle the work tree'; e2e_expect $n 'fix: greeting'; e2e_reject $n 'Unstaged changes'
sc_quit $n
printf 'dirty\n' >> $d/repo/README.md
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -w
e2e_wait_for $n 'README\.md'; e2e_expect $n 'dirty'
sc_quit $n
