#!/usr/bin/env bash
# Open PR #1 through the fake gh, see title + a diff line, quit.
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
n=$(e2e_name); d=$(e2e_dir "$n"); mkdir -p "$d"
fx_pr_repo "$d/repo"
E2E_CWD="$d/repo" e2e_start "$n" "$TUICR_BIN" pr 1
e2e_wait_for "$n" 'Add greeting and tweak README'
e2e_expect "$n" 'hello from the PR'
e2e_forge_expect '"pr", "view", "1"'
e2e_keys "$n" :q Enter   # `q` alone is only a hint
e2e_wait_exit "$n"
