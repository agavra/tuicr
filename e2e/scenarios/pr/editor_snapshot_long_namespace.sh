#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers the snapshot cache dir name limit: with a 260-char owner the old `<slug>-pr1-<short>` dir exceeded NAME_MAX, so `e`
# failed to open the snapshot at all. Same private-snapshot checks as editor_snapshot_private_dir.
sc_pr_editor_snapshot "$(e2e_name)" "https://github.com/$(printf 'n%.0s' $(seq 260))/demo.git"
