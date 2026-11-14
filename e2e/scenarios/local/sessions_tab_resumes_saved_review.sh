#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Sessions tab: a saved review is listed with its comment count and Enter resumes it (target and comments are back).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n Enter
e2e_wait_for $n 'unstaged line'; sc_cursor_on $n 'unstaged line'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'resume me'; e2e_keys $n Enter; e2e_wait_for $n 'resume me'
sc_quit $n
E2E_CWD=$d/repo e2e_start_keep $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'
e2e_keys $n Tab Tab; e2e_wait_for $n 'Sessions'
e2e_wait_for $n '1 comments'
e2e_keys $n Enter
e2e_wait_for $n 'unstaged line'; e2e_wait_for $n 'resume me'
sc_quit $n
