#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# :e reloads the working-tree diff; comments + reviewed marks survive a restart.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n Enter
e2e_wait_for $n 'unstaged line'
# modify the file on disk, :e picks it up
printf 'late edit from outside\n' >> $d/repo/README.md
e2e_reject $n 'late edit from outside' 1
sc_cmd $n e
e2e_wait_for $n 'late edit from outside'
# comment + reviewed, save, quit
sc_cursor_on $n 'late edit from outside'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'persist me'; e2e_keys $n Enter; e2e_wait_for $n 'persist me'
e2e_keys $n '{'; e2e_keys $n r; e2e_wait_for $n 'Files · 1/2'
sc_save $n
sc_cmd $n 'q!'; e2e_wait_exit $n
# restart in the same repo: same selection, comment and reviewed mark come back
E2E_CWD=$d/repo e2e_start_keep $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n Enter
e2e_wait_for $n 'Files · 1/2'; e2e_expect $n '✓ README\.md'
sc_cmd $n summary; e2e_wait_for $n 'SUMMARY'; e2e_expect $n 'persist me'
e2e_keys $n Escape
sc_quit $n
