#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Opening ONE historical commit straight from the startup selector (Enter on a commit row, no Space range) reviews
# exactly that commit: its message and its own files, nothing from later commits or from the working tree.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'
e2e_keys $n j j j                          # rows: unstaged, staged, fix, feat, base -> cursor on "feat"
e2e_keys $n Enter
e2e_wait_for $n 'Commit Message \([0-9a-f]+\)'
e2e_expect $n 'feat: touch every file kind'; e2e_expect $n 'Files · 0/8'
e2e_expect $n '▢ M README\.md'; e2e_expect $n '▢ D gone\.txt'
e2e_keys $n Escape
e2e_expect $n 'second line'                # README.md's change from this very commit
e2e_reject $n 'unstaged line'              # working-tree edit
e2e_reject $n 'staged\.txt'
e2e_reject $n 'fix: greeting'              # a later commit
e2e_reject $n 'src/main\.rs'               # only touched by the later commit
sc_quit $n
