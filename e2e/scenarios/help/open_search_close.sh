#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# ? opens help, / searches it, q closes.
n=$(e2e_name); sc_diff $n
e2e_keys $n '?'; e2e_wait_for $n 'HELP|Help|Keybindings'
e2e_expect $n 'reviewed|comment'
e2e_keys $n '/'; e2e_keys $n 'visual'; e2e_keys $n Enter; e2e_settle $n
e2e_keys $n '?'
e2e_wait_gone $n 'HELP'
e2e_expect $n 'Files · 0/8'
sc_cmd $n help; e2e_wait_for $n 'HELP|Help'
e2e_keys $n q; e2e_wait_gone $n 'HELP'
sc_quit $n
