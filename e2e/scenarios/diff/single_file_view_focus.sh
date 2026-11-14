#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `:focus` shows one file at a time: `}` / `{` swap the file under review; `:focus` again returns to the continuous diff.
n=$(e2e_name); sc_diff $n
e2e_wait_for $n 'delete me'                                          # continuous: README.md and gone.txt both rendered
sc_cmd $n focus; e2e_wait_for $n 'FOCUS'
e2e_wait_gone $n 'delete me'; e2e_expect $n 'second line'
e2e_keys $n '}'; e2e_wait_for $n '\(binary file\)'; e2e_reject $n 'second line'
e2e_keys $n '}'; e2e_wait_for $n 'delete me'; e2e_reject $n 'binary file'
e2e_keys $n '{'; e2e_keys $n '{'; e2e_wait_for $n 'second line'
sc_cmd $n focus; e2e_wait_gone $n 'FOCUS'; e2e_wait_for $n 'delete me'; e2e_expect $n 'second line'
sc_quit $n
