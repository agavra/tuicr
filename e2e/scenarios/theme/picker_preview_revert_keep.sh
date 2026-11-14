#!/usr/bin/env bash
# :theme opens the picker; j previews live (colours change), Esc reverts, Enter keeps; :theme <name> applies directly.
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
n=$(e2e_name); sc_diff $n
# colour-aware snapshot of the whole pane, minus the popup rows (compare only the status line + header rows)
ansi() { e2e_colors_hash $n '1p;45p'; }
base=$(ansi)
sc_cmd $n theme
e2e_wait_for $n 'Switch Theme'; e2e_expect $n 'gruvbox-dark'; e2e_expect $n 'nord-dark'
p0=$(ansi)
e2e_keys $n j; e2e_settle $n; e2e_keys $n j; e2e_settle $n
p2=$(ansi)
[ "$p0" != "$p2" ] || { echo "live preview did not change colours"; exit 1; }
e2e_keys $n Escape; e2e_wait_gone $n 'Switch Theme'
e2e_settle $n; [ "$(ansi)" = "$base" ] || { echo "Esc did not revert the preview"; exit 1; }
# Enter keeps the previewed theme
sc_cmd $n theme; e2e_wait_for $n 'Switch Theme'; e2e_keys $n j; e2e_settle $n; e2e_keys $n Enter
e2e_wait_gone $n 'Switch Theme'; e2e_settle $n
[ "$(ansi)" != "$base" ] || { echo "Enter did not keep the theme"; exit 1; }
# direct name, and unknown name
sc_cmd $n 'theme nord-dark'; e2e_settle $n; e2e_expect $n 'Files · 0/8'
sc_cmd $n 'theme no-such-theme-xyz'
e2e_wait_for $n '[Uu]nknown|not found|[Ii]nvalid|No theme|no such'
sc_quit $n
