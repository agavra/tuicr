#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# -r A..B, -r A...B (merge-base forms) and the default -r HEAD~2..HEAD.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d
fx_git_merge_base $d/mb
E2E_CWD=$d/mb e2e_start $n "$TUICR_BIN" --no-update-check -r 'main...topic'
e2e_wait_for $n 'topic\.txt'; e2e_expect $n 'topic change'; e2e_reject $n 'mainonly'
e2e_keys $n ':q!' Enter; e2e_wait_exit $n
# the other side
E2E_CWD=$d/mb e2e_start $n "$TUICR_BIN" --no-update-check -r 'topic...main'
e2e_wait_for $n 'mainonly\.txt'; e2e_expect $n 'main change'; e2e_reject $n 'topic\.txt'
e2e_keys $n ':q!' Enter; e2e_wait_exit $n
# two-dot, explicit shas
E2E_CWD=$d/mb e2e_start $n "$TUICR_BIN" --no-update-check -r "$FX_BASE..$FX_TOPIC"
e2e_wait_for $n 'topic\.txt'; e2e_expect $n 'topic change'
e2e_keys $n ':q!' Enter; e2e_wait_exit $n
# multi-commit range on the 3-commit repo
fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r 'HEAD~2..HEAD'
e2e_wait_for $n 'Files · 0/8'; e2e_expect $n 'fix: greeting'; e2e_expect $n 'feat: touch every file kind'; e2e_reject $n '1f95fd7|\bbase\b'
e2e_keys $n ':q!' Enter; e2e_wait_exit $n
