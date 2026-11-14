#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `-p PATH` reviews only that path of the working tree (staged and untracked files elsewhere are left out).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -p README.md
e2e_wait_for $n 'README\.md \[M\]'; e2e_expect $n 'unstaged line'   # a single file: no tree pane
e2e_reject $n 'staged\.txt|untracked\.txt'
sc_quit $n
