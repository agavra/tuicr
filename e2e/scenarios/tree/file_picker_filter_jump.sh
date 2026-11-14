#!/usr/bin/env bash
# <leader>p opens the file picker; / filters; Enter jumps; Esc closes without moving.
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
n=$(e2e_name); sc_diff $n
e2e_keys $n ';p'; e2e_wait_for $n 'Go to File \(8/8\)'
e2e_keys $n '/'; e2e_keys $n 'tabs'
e2e_wait_for $n 'Go to File \(1/8\)'
e2e_keys $n Enter; e2e_wait_gone $n 'Go to File'; e2e_wait_for $n '┌ tabs\.txt ─'
# Esc closes without moving
e2e_keys $n ';p'; e2e_wait_for $n 'Go to File \(8/8\)'; e2e_keys $n j Escape
e2e_wait_gone $n 'Go to File'; e2e_expect $n '┌ tabs\.txt ─'
# j then Enter jumps to the selected file
e2e_keys $n ';p'; e2e_wait_for $n 'Go to File'; e2e_keys $n j Enter
e2e_wait_gone $n 'Go to File'; e2e_wait_for $n '┌ src/lib\.rs ─'
sc_quit $n
