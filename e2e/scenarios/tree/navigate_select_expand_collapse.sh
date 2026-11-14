#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# File tree basics with the tree focused (<leader>h): j/k move the selection and Enter jumps the diff to the selected file;
# Space or Enter toggles the directory under the selection, O collapses and o expands every directory; child rows appear and
# disappear accordingly. Enter on a file moves focus to the diff, so each step refocuses the tree first.
n=$(e2e_name); sc_diff $n
cur() { e2e_wait_for $n "┌ $1 ─+ "; }              # the diff pane title names the file the diff is on
row='▢ M lib\.rs'                                      # src/lib.rs as a tree row (the diff only shows the full path)
tree() { e2e_keys $n ';h'; e2e_settle $n; }
tree; e2e_keys $n Enter; cur 'README\.md'                   # selection starts on the first file
tree; e2e_keys $n j j Enter; cur 'gone\.txt'                # data.bin, gone.txt
tree; e2e_keys $n k Enter; cur 'data\.bin'
tree; e2e_keys $n j j j j j; e2e_expect $n "$row"           # gone, new_name, notes_cjk, tabs, then the src/ directory row
# Space on the directory collapses it, Space again expands it; Enter on it toggles too and does not leave the tree
e2e_keys $n Space; e2e_wait_gone $n "$row"; e2e_expect $n '▶ src/'
e2e_keys $n Space; e2e_wait_for $n "$row"; e2e_expect $n '▼ src/'
e2e_keys $n Enter; e2e_wait_gone $n "$row"; e2e_expect $n '▶ src/'
e2e_keys $n Enter; e2e_wait_for $n "$row"; e2e_expect $n '▼ src/'; cur 'data\.bin'
# a child is reachable with j + Enter
e2e_keys $n j Enter; cur 'src/lib\.rs'
# O collapses every directory, o expands them all again
tree; e2e_keys $n O; e2e_wait_gone $n "$row"; e2e_wait_gone $n '▢ M main\.rs'; e2e_expect $n '▶ src/'
e2e_keys $n o; e2e_wait_for $n "$row"; e2e_expect $n '▢ M main\.rs'
sc_quit $n
