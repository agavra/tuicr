#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# :diff toggles side-by-side and back.
n=$(e2e_name); sc_diff $n
sc_cmd $n diff
# SBS: old and new text of the same change on one row, split by a divider
e2e_wait_for $n '3  first line +│ +3  first line'
sc_cmd $n diff
e2e_wait_gone $n '3  first line +│ +3  first line'
e2e_expect $n '4 ▌ second line'
sc_quit $n
