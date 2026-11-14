#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #888: file tree: horizontal scroll moves every row, including its indent and status glyph.
# Baseline pins each row's indent while only the name scrolls (so `an_extremely_...` keeps 4 leading spaces and `b.txt` 2).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_wide_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~1..HEAD
e2e_wait_for $n 'Files · 0/4'; e2e_keys $n Escape
sc_cmd $n 'set nowrap'; sc_cmd $n diff
e2e_keys $n ';h'; e2e_settle $n 0.2; e2e_keys $n ';h'; e2e_settle $n 0.2   # new side -> old side -> file list
e2e_keys $n l; e2e_settle $n
# scrolled by 4 columns: the nested file row now starts at its glyph/status, nothing pins the indent
e2e_wait_for $n '^│▢ M an_extremely_long_file_nam' || { echo "file row not scrolled uniformly (indent pinned)"; e2e_screen $n; exit 1; }
e2e_expect $n '^│M b\.txt ' || { echo "b.txt row not scrolled uniformly"; exit 1; }
sc_quit $n
