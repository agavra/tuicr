#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../../lib/init.sh"
# Runner self-test (expect PASS): a scenario that provokes unexpected calls on purpose declares it.
n=$(e2e_name)
e2e_allow_unexpected_forge_calls
e2e_start $n sh -c 'gh api repos/octo/demo/nonexistent >/dev/null 2>&1; sleep 30'
e2e_wait_forge_log 'UNEXPECTED' 10
