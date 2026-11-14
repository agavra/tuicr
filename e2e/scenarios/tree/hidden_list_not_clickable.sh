#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# after hiding the file list (<leader>e) its old rectangle must stop being a click target.
# Baseline kept the stale hit area, so clicking where the tree used to be selected a file instead of landing in the diff.
n=$(e2e_name)
sc_diff $n
e2e_keys $n -l ';e'
e2e_wait_gone $n 'Files · '
# SGR mouse press+release at (col 5,row 9) = where `gone.txt` sat in the tree
e2e_keys $n -l "$(printf '\033[<0;5;9M\033[<0;5;9m')"; e2e_settle $n
e2e_expect $n '┌ README.md '
e2e_reject $n '┌ gone.txt '
# the click lands in the diff: cursor row moved onto `1   # demo`
e2e_screen $n | grep -Eq '▶ +1 +# demo' || { echo "click did not move the diff cursor"; exit 1; }
sc_quit $n
