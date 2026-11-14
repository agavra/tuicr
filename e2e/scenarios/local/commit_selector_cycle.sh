#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Commit selector: Space range, Enter confirm, ( ) cycle individual commits in the inline selector.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'
e2e_keys $n j j Space Space Enter      # rows: unstaged, staged, fix, feat, base; Space advances -> fix + feat
e2e_wait_for $n 'Files · 0/8'; e2e_expect $n 'commits/[0-9a-f]+\.\.[0-9a-f]+'; e2e_expect $n 'data\.bin'
e2e_expect $n 'fix: greeting'; e2e_expect $n 'feat: touch every file kind'
# ) / ( cycle: first commit alone, ..., then back to the whole range. A lone commit shows its message.
e2e_keys $n ')'; e2e_wait_for $n 'Commit Message \([0-9a-f]+\)'; e2e_expect $n 'Files · 0/8'   # feat alone
e2e_keys $n ')'; e2e_wait_gone $n 'Commit Message \('; e2e_expect $n '▢ M README\.md'            # whole range again
e2e_keys $n '('; e2e_wait_for $n 'Commit Message \([0-9a-f]+\)'; e2e_expect $n 'Files · 0/2'    # fix alone
e2e_expect $n 'fix: greeting'
e2e_keys $n '('; e2e_wait_gone $n 'Commit Message \('; e2e_expect $n 'Files · 0/8'
# Enter narrows to the commit under the cursor (cursor is on the first row: fix)
e2e_keys $n ';k'; e2e_keys $n Enter; e2e_wait_for $n 'Commit Message \('; e2e_expect $n 'Files · 0/2'
# Space widens the selection again
e2e_keys $n j Space; e2e_wait_gone $n 'Commit Message \('; e2e_expect $n 'Files · 0/8'
sc_quit $n
