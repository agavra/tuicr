#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `:set noreviewed` hides files marked reviewed (tree, diff); `r` then moves on to the next file; the tree title keeps
# counting them; a file whose hunk is only `R`-marked stays; `:set reviewed` brings them back.
n=$(e2e_name); sc_diff $n
e2e_keys $n g; e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'       # first file is reviewed (still shown)
e2e_expect $n '▣ M README\.md'                                        # reviewed rows carry ▣
sc_cmd $n 'set noreviewed'
e2e_wait_gone $n '▣ M README\.md'
e2e_wait_for $n 'Files · 1/8'                                      # the progress fraction still counts the hidden file
e2e_wait_for $n 'reviewed hidden'
e2e_keys $n r; e2e_wait_for $n 'Files · 2/8'                        # burn-down: marking the file under the cursor hides it
e2e_wait_gone $n 'M data\.bin'; e2e_expect $n 'D gone\.txt'   # data.bin left the tree; the next file is up
# a hunk-level `R` mark alone does not hide its file
e2e_keys $n R; e2e_settle $n; e2e_expect $n 'D gone\.txt'; e2e_wait_for $n 'Files · 2/8'
e2e_keys $n ':reviewed' Enter                                       # bare :reviewed toggles back to showing
e2e_wait_for $n '▣ M README\.md'; e2e_reject $n 'reviewed hidden'
sc_quit $n
