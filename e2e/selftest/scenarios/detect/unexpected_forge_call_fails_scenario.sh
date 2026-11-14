#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../../lib/init.sh"
# Runner self-test (expect FAIL): the scenario body "succeeds", but a fake forge saw a call it has no fixture for.
# tuicr treats some forge failures as best-effort, so only the central check can catch this.
n=$(e2e_name)
e2e_start $n sh -c 'gh api repos/octo/demo/nonexistent >/dev/null 2>&1; sleep 30'
e2e_wait_forge_log 'UNEXPECTED' 10
