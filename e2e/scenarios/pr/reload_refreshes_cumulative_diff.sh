#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# #868: a same-head reload while the cumulative diff is shown must refresh the cached cumulative
# diff, so narrowing to a commit and widening back shows the reloaded diff, not the stale one.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; echo threads_and_commits > $d/variant
E2E_ENV="E2E_VARIANT_FILE=$d/variant" sc_pr_open $n
e2e_keys $n ';k'; e2e_keys $n j Enter                 # narrow to bbbbbbb
e2e_wait_for $n 'Commit Message \(bbbbbbb\)'
e2e_keys $n ')'; e2e_wait_for $n 'Commit Message \(aaaaaaa\)'
e2e_keys $n ')'; e2e_wait_gone $n 'Commit Message \('; e2e_wait_for $n '\+?.*new line'   # whole PR again
echo cumulative_refresh > $d/variant                           # same head, but the cumulative diff changed
sc_cmd $n e
e2e_wait_for $n 'newer line'
e2e_keys $n ';k'; e2e_keys $n k k k; e2e_settle $n; e2e_keys $n Enter
e2e_wait_for $n 'Commit Message \(2222222\)'
e2e_keys $n ')'; e2e_wait_for $n 'Commit Message \(bbbbbbb\)'
e2e_keys $n ')'; e2e_wait_for $n 'Commit Message \(aaaaaaa\)'
e2e_keys $n ')'; e2e_wait_gone $n 'Commit Message \('
e2e_reject $n 'Commit Message \(' || { echo "not back on the whole PR"; exit 1; }
e2e_wait_for $n 'newer line' || { echo "widened selection shows the stale cumulative diff"; exit 1; }
sc_quit $n
