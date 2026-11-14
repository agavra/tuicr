#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../../lib/init.sh"
# Runner self-test (expect PASS): wait_exit verifies the exit status (default 0); a crash is not "exited fine".
n=$(e2e_name)
e2e_start $n sh -c 'exit 3'
! e2e_wait_exit $n 5 2>/dev/null || { echo "wait_exit accepted status 3 as success"; exit 1; }
e2e_wait_exit $n 5 3                       # the expected status passes
e2e_wait_exit $n 5 any
m=${n}_ok
e2e_start $m sh -c 'exit 0'
e2e_wait_exit $m 5
