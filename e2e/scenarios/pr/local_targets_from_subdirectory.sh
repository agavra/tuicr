#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #916: `tuicr pr 1` started from a subdirectory must still scope the Sessions tab / local targets by the
# checkout root. Baseline stored the raw cwd (src/), so a session saved at the checkout root was not listed.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo; _fx_git $d/repo remote add origin https://github.com/octo/demo.git
# 1. create a local session at the root (comment on the unstaged diff), save, quit
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n Enter; e2e_wait_for $n 'unstaged line'
sc_cursor_on $n 'unstaged line'; e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n -l 'root-session-comment'; e2e_keys $n Enter; e2e_wait_for $n 'root-session-comment'
sc_save $n; sc_cmd $n 'q!'; e2e_wait_exit $n
# 2. PR from src/, then the Sessions tab
E2E_KEEP=1 E2E_ENV="E2E_VARIANT=rich" E2E_CWD=$d/repo/src e2e_start $n "$TUICR_BIN" --no-update-check pr 1
e2e_wait_for $n 'PR Description' 20
sc_cmd $n commits; e2e_wait_for $n 'Unstaged changes'
e2e_keys $n Tab; e2e_keys $n Tab
e2e_expect $n 'Sessions' || exit 1
e2e_expect $n 'root-session|unstaged|Unstaged' || { echo "Sessions tab did not list the checkout's local session from a subdirectory launch"; exit 1; }
e2e_keys $n Escape; sc_quit $n
