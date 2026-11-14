#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `:set relativenumber` numbers rows by distance from the cursor (the cursor row keeps its own line number); `;n` and
# `:set norelativenumber` restore absolute numbers.
n=$(e2e_name); sc_diff $n
sc_cursor_on $n '4 ▌ second line'
e2e_expect $n '^│.*│ +3 +first line'                                  # absolute: line 3
sc_cmd $n 'set relativenumber'
e2e_wait_for $n '│ +1 +first line'; e2e_expect $n '│ +3 +# demo'; e2e_expect $n '▶ +4 ▌ second line'
sc_cmd $n 'set norelativenumber'
e2e_wait_for $n '│ +3 +first line'; e2e_expect $n '│ +1 +# demo'
e2e_keys $n ';n'; e2e_wait_for $n '│ +1 +first line'                  # <leader>n toggles it on
e2e_keys $n ';n'; e2e_wait_for $n '│ +3 +first line'
sc_quit $n
