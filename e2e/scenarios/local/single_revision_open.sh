#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `-r <rev>` with a single revision (a ref, then a full SHA) reviews that commit against its parent, not the history up to it.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD
e2e_wait_for $n 'Commit Message \([0-9a-f]+\)'; e2e_expect $n 'fix: greeting'; e2e_expect $n 'Files · 0/2'
e2e_expect $n 'src/main\.rs'
e2e_reject $n 'README\.md'; e2e_reject $n 'feat: touch every file kind'   # an earlier commit
e2e_keys $n Escape; e2e_expect $n 'hello'
sc_quit $n
# an explicit SHA, of the middle commit: its own seven files + message, not the later commit's
sha=$(git -C $d/repo rev-parse HEAD~1)
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r "$sha"
e2e_wait_for $n 'Commit Message \([0-9a-f]+\)'; e2e_expect $n 'feat: touch every file kind'; e2e_expect $n 'Files · 0/8'
e2e_expect $n '▢ M README\.md'; e2e_expect $n '▢ D gone\.txt'
e2e_reject $n 'fix: greeting'; e2e_reject $n 'src/main\.rs'
sc_quit $n
