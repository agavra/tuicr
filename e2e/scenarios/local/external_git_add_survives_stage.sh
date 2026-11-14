#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# a `git add` done outside tuicr while it runs must stay staged after :stage.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'
e2e_keys $n Enter
e2e_wait_for $n '▢ M README\.md'
# external staging after tuicr already read the index
printf 'external\n' > $d/repo/ext.txt; git -C $d/repo add ext.txt
git -C $d/repo diff --cached --name-only | grep -qx ext.txt
e2e_keys $n Escape; e2e_keys $n j                # focus file tree, README row
e2e_keys $n r; e2e_wait_for $n 'Files · 1/'
sc_cmd $n stage
# barrier: :stage wrote README.md into the index
e2e_wait_cmd 10 bash -c "git -C $d/repo diff --cached --name-only | grep -qx README.md"; e2e_settle $n
cached=$(git -C $d/repo diff --cached --name-only)
echo "cached: $cached"
grep -qx 'README.md' <<<"$cached" || { echo "README.md was not staged"; e2e_screen $n; exit 1; }
grep -qx 'ext.txt' <<<"$cached" || { echo "external git add (ext.txt) was lost by :stage"; exit 1; }
grep -qx 'staged.txt' <<<"$cached" || { echo "staged.txt was lost by :stage"; exit 1; }
sc_quit $n
