#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../../lib/init.sh"
# Runner self-test (expect FAIL, rc 124): a hung scenario is killed at E2E_SCENARIO_TIMEOUT together with its background children.
sleep 300 &
echo $! > "${E2E_ROOT%/h}/hung_child.pid"
sleep 300
