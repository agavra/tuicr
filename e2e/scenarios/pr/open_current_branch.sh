#!/usr/bin/env bash
# `tuicr pr` with no target opens the PR whose head is the checked-out branch (fake gh lists PR #1 on feature/greet).
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
n=$(e2e_name); d=$(e2e_dir "$n"); mkdir -p "$d"
fx_pr_repo "$d/repo"
git -C "$d/repo" checkout -q -b feature/greet
E2E_CWD="$d/repo" e2e_start "$n" "$TUICR_BIN" --no-update-check pr
e2e_wait_for "$n" 'Add greeting and tweak README'
e2e_expect "$n" 'hello from the PR'
e2e_forge_expect '"pr", "list"'
e2e_forge_expect '"pr", "view", "1"'
e2e_keys "$n" :q Enter   # `q` alone is only a hint
e2e_wait_exit "$n"
