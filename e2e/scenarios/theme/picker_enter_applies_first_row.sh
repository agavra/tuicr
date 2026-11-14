#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #901: `:theme` then Enter without moving must apply the selected (first) row. Baseline only announced it
# ("Theme: dark") and kept the running theme. Start on --theme light; first picker row is `dark`.
n=$(e2e_name)
sc_open $n --theme light -r HEAD~2..HEAD; e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
ansi() { e2e_colors_hash $n '1p;3,30p'; }
base=$(ansi)
sc_cmd $n theme; e2e_wait_for $n 'Switch Theme'
e2e_keys $n Enter; e2e_wait_gone $n 'Switch Theme'; e2e_settle $n
e2e_screen $n | tail -2
[ "$(ansi)" != "$base" ] || { echo "Enter on the first picker row did not apply that theme"; exit 1; }
sc_quit $n
