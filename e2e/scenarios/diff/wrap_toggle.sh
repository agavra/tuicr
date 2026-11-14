#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# :set wrap! wraps long lines; toggling back truncates them again.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
long=$(printf 'word%.0s ' $(seq 1 60))END
printf '%s\n' "$long" > $d/repo/long.txt; git -C $d/repo add long.txt
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n j Enter
e2e_wait_for $n 'long\.txt'
sc_cmd $n 'set wrap'
e2e_wait_for $n 'END'                  # wrapped: the tail of the long line is visible
sc_cmd $n 'set wrap!'
e2e_wait_gone $n 'END'                 # toggled off: truncated
sc_cmd $n 'wrap'
e2e_wait_for $n 'END'                  # :wrap toggles back on
sc_quit $n
