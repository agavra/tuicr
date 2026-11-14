#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# config.toml: `file_list_position = "right"` puts the file list right of the diff, `leader = ","` replaces `;` for the sidebar
# toggle, `q_quits = true` makes a bare `q` quit; `show_file_list = false` starts with the list hidden.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/config/tuicr; fx_git_repo $d/repo
printf 'file_list_position = "right"\nleader = ","\nq_quits = true\n' > $d/config/tuicr/config.toml
E2E_CWD=$d/repo e2e_start_keep $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
col=$(e2e_screen $n | grep -m1 'Files · 0/8' | awk '{print index($0, "Files · 0/8")}')
[ "$col" -gt 100 ] || { echo "file list is not on the right (column $col)"; e2e_screen $n; exit 1; }
e2e_keys $n ',e'; e2e_wait_gone $n 'Files · 0/8'                       # the configured one hides the list
e2e_keys $n ',e'; e2e_wait_for $n 'Files · 0/8'
e2e_keys $n q; e2e_wait_exit $n                                         # bare q quits
m=${n}_hidden; mkdir -p $(e2e_dir $m)/config/tuicr; printf 'show_file_list = false\n' > $(e2e_dir $m)/config/tuicr/config.toml
E2E_CWD=$d/repo e2e_start_keep $m "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
e2e_wait_for $m 'second line'; e2e_reject $m 'Files · 0/8'
e2e_keys $m ';e'; e2e_wait_for $m 'Files · 0/8'
e2e_keys $m q; e2e_wait_for $m 'q no longer quits'                                  # q_quits is off here: a hint, no quit
e2e_alive $m || { echo "q quit with q_quits unset"; exit 1; }
sc_quit $m
