#!/usr/bin/env bash
# docs/CONFIG.md documents `wrap` default = false: long lines must be truncated on a fresh start.
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
python3 -c "print('word '*60+'END')" > $d/repo/long.txt; git -C $d/repo add long.txt
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n j Enter
e2e_wait_for $n 'long\.txt'; e2e_wait_for $n 'word word'
e2e_reject $n 'END' 1
e2e_keys $n ':q!' Enter; e2e_wait_exit $n
