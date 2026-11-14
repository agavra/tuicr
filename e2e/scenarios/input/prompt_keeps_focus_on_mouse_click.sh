#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #900: with the file-tree `i` prompt open, a mouse press in the diff must not steal input from the prompt.
# Baseline moved panel focus to the diff while the prompt still captured keys, so Esc hid the sidebar (file list) instead of
# cancelling the prompt.
n=$(e2e_name)
sc_diff $n
e2e_keys $n -l ';h'; e2e_settle $n 0.2; e2e_keys $n -l ';h'; e2e_settle $n 0.2
e2e_keys $n i; e2e_wait_for $n '└ i ' 3
e2e_keys $n -l 'ab'; e2e_wait_for $n '└ i ab' 3
# SGR press+release in the diff pane (col 60,row 20)
e2e_keys $n -l "$(printf '\033[<0;60;20M\033[<0;60;20m')"; e2e_settle $n
e2e_keys $n Escape; e2e_settle $n
e2e_expect $n 'Files · 0/8' || { echo "Esc after a diff click hid the file list instead of cancelling the prompt"; exit 1; }
e2e_reject $n '└ i ab' 1
sc_quit $n
