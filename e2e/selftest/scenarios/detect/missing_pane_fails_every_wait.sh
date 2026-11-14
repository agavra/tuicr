#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../../lib/init.sh"
# Runner self-test (expect PASS): with no pane at all, every screen/process query must FAIL. Before the refactor a
# missing pane counted as "text disappeared", "forbidden text absent" and "process exited" (all returned 0).
n=$(e2e_name)
! e2e_wait_for $n 'anything' 1 2>/dev/null || { echo "wait_for succeeded without a pane"; exit 1; }
! e2e_wait_gone $n 'anything' 1 2>/dev/null || { echo "wait_gone succeeded without a pane"; exit 1; }
! e2e_reject $n 'anything' 1 2>/dev/null || { echo "reject succeeded without a pane"; exit 1; }
! e2e_wait_exit $n 1 2>/dev/null || { echo "wait_exit succeeded without a pane"; exit 1; }
! e2e_settle $n 0.2 1 2>/dev/null || { echo "settle succeeded without a pane"; exit 1; }
! e2e_screen $n >/dev/null || { echo "screen succeeded without a pane"; exit 1; }
! e2e_alive $n || { echo "alive is true without a pane"; exit 1; }
echo "all six queries failed on a missing pane, as required"
