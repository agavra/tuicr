#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../../lib/init.sh"
# Runner self-test (expect QFAIL via the selftest quarantine file): a known failure is reported, not gating.
exit 1
