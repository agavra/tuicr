#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #901: preview a local theme, delete its file, then confirm. Confirm must fail, restore the original theme
# and show an error. Baseline announced "Theme: zz-local" and kept the previewed colours.
n=$(e2e_name)
sc_open $n; d=$(e2e_dir $n); e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
mkdir -p $d/config/tuicr/themes
sed 's/^syntax_theme.*//' "$E2E_DIR/../examples/tuicr-teal.toml" > $d/config/tuicr/themes/zz-local.toml
ansi() { e2e_colors_hash $n '1p;3,30p'; }
base=$(ansi)
sc_cmd $n theme; e2e_wait_for $n 'Switch Theme'
e2e_keys $n /; e2e_keys $n -l zz-local; e2e_keys $n Enter; e2e_settle $n 0.4
[ "$(ansi)" != "$base" ] || { echo "local theme preview did not change colours"; e2e_screen $n; exit 1; }
rm $d/config/tuicr/themes/zz-local.toml
e2e_keys $n Enter; e2e_wait_for $n "Unknown theme 'zz-local'"; e2e_settle $n 0.4
e2e_reject $n 'Theme: zz-local' 0.3 || { echo "announced a theme that failed to apply"; exit 1; }
[ "$(ansi)" = "$base" ] || { echo "failed confirm did not restore the original theme"; exit 1; }
sc_quit $n
