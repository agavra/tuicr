#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Sessions tab: `dd` on a saved review asks first (`n` keeps it), `y` deletes it and its session file.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_keys $n Enter
e2e_wait_for $n 'unstaged line'; sc_cursor_on $n 'unstaged line'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'delete me'; e2e_keys $n Enter; e2e_wait_for $n 'delete me'
sc_quit $n
E2E_CWD=$d/repo e2e_start_keep $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'
e2e_keys $n Tab Tab; e2e_wait_for $n '1 comments'
e2e_keys $n d d; e2e_wait_for $n 'Delete this saved review\?'
e2e_keys $n n; e2e_wait_gone $n 'Delete this saved review'
e2e_expect $n '1 comments'
sc_session_has $n 'delete me' || { echo "declining the prompt deleted the session"; exit 1; }
e2e_keys $n d d; e2e_wait_for $n 'Delete this saved review\?'
e2e_keys $n y
e2e_wait_gone $n '1 comments'
! sc_session_has $n 'delete me' || { echo "dd left the session file"; exit 1; }
sc_quit $n
