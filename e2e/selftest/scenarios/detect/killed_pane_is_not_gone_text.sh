#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../../lib/init.sh"
# Runner self-test (expect PASS): a pane that was killed mid-scenario must not satisfy "text is gone".
n=$(e2e_name)
e2e_start $n sh -c 'echo visible-text; sleep 60'
e2e_wait_for $n 'visible-text' 5
e2e_wait_gone $n 'not-on-screen' 2        # control: a live pane without the text passes
_tmux kill-session -t "$(_e2e_sess $n)"
! e2e_wait_gone $n 'visible-text' 1 2>/dev/null || { echo "wait_gone passed on a killed pane"; exit 1; }
! e2e_reject $n 'forbidden' 1 2>/dev/null || { echo "reject passed on a killed pane"; exit 1; }
