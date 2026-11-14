#!/usr/bin/env bash
# Open the git fixture, see the file tree + a diff, quit with :q.
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
n=$(e2e_name); d=$(e2e_dir "$n"); mkdir -p "$d"
fx_git_repo "$d/repo"
E2E_CWD="$d/repo" e2e_start "$n" "$TUICR_BIN"
e2e_wait_for "$n" 'Staged changes|Unstaged changes'   # commit selector
e2e_keys "$n" Enter
e2e_expect "$n" 'README\.md'                            # file tree
e2e_expect "$n" 'unstaged line'                         # diff content
e2e_keys "$n" :q Enter   # `q` alone is only a hint
e2e_wait_exit "$n"
