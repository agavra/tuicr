#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #900: a bracketed paste into the file-tree `i` filter prompt lands in the draft (`└ i README` on the tree border).
# Baseline's paste table only knew comment/command/search modes and silently dropped the paste.
n=$(e2e_name)
sc_diff $n
e2e_keys $n -l ';h'; e2e_settle $n 0.2; e2e_keys $n -l ';h'; e2e_settle $n 0.2
e2e_keys $n i; e2e_wait_for $n '└ i ' 3
_tmux set-buffer -b e2e "README"; _tmux paste-buffer -p -b e2e -t "$(_e2e_sess $n)"
e2e_expect $n '└ i README' 3 || { echo "pasted text did not reach the file-tree filter prompt"; exit 1; }
e2e_keys $n Enter
e2e_expect $n 'Files · 0/1' 3
sc_quit $n
