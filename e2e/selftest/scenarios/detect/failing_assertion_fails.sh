#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../../lib/init.sh"
# Runner self-test (expect FAIL): text that is not on screen is a failure, with the screen in the log.
n=$(e2e_name)
e2e_start $n sh -c 'echo hello; sleep 60'
e2e_wait_for $n 'hello' 5
e2e_expect $n 'goodbye' 1
