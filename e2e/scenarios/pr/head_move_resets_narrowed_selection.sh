#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# #867: when the PR gets a new head with a changed commit list, `:e` resets a narrowed
# selection to the whole PR (no stale commit-message pseudo file above the cumulative diff),
# and the reviewed mark of a surviving commit's message is carried to the new head.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; echo threads_and_commits > $d/variant
E2E_ENV="E2E_VARIANT_FILE=$d/variant" sc_pr_open $n
e2e_keys $n ';k'
e2e_keys $n j Enter                                  # narrow to bbbbbbb (second commit)
e2e_wait_for $n 'Commit Message \(bbbbbbb\)'; e2e_wait_for $n 'line from commit B'
e2e_keys $n Escape; e2e_keys $n g                     # diff pane, top = commit message entry
e2e_keys $n r; e2e_wait_for $n '✓.*Commit Message \(bbbbbbb\)'
# the PR gets a new head and a fourth commit
echo moved_head > $d/variant
sc_cmd $n e
# the reload resets the selection to the cumulative diff: the single-commit message above it goes away
e2e_wait_gone $n 'Commit Message \(' 15; e2e_wait_for $n 'hello from the PR'; e2e_reject $n 'Commit Message \('
scr=$(e2e_screen $n); grep -Eq '▣.*4444444|4444444.*fourth|fourth commit' <<<"$scr" || { echo "new commit not listed"; e2e_screen $n; exit 1; }
# narrow to bbbbbbb again: its message review must have been carried over
e2e_keys $n ';k'; e2e_keys $n j j; e2e_wait_for $n '▸.*bbbbbbb'; e2e_keys $n Enter
e2e_wait_for $n 'Commit Message \(bbbbbbb\)'
e2e_screen $n | grep -Eq '✓.*Commit Message \(bbbbbbb\)' || { echo "reviewed mark of the commit message was not carried"; e2e_screen $n; exit 1; }
sc_quit $n
