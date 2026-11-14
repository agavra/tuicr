#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers the editor snapshot dir: `e` on a PR file the checkout lacks opens a read-only snapshot. It must live in a private
# (0700, user-owned) base directory named by the full head sha, not in a world-readable $TMPDIR/tuicr/snapshots/<repo>-pr<N>-<short>.
sc_pr_editor_snapshot "$(e2e_name)"
