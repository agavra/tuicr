#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #893: head keeps moving (fake gh: a new head after every data call).
# Fixed: the open fails with "changed while it was being loaded; try again" and no PR is shown.
# Baseline: opens PR #1 anyway with a mismatched head/diff pair.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_pr_repo $d/repo
# stderr (where the exit error goes once the TUI is torn down) is captured to a file
E2E_ENV="E2E_HEAD_MODE=every" E2E_CWD="$d/repo" e2e_start $n bash -c "$TUICR_BIN --no-update-check pr 1 2>$d/stderr.txt"
e2e_wait_exit $n 20 1 || { echo "PR opened despite the head moving on every request"; exit 1; }
grep -q 'changed while it was being loaded; try again' $d/stderr.txt || { echo "no head-moved error:"; cat $d/stderr.txt; exit 1; }
# fetched twice (one retry), then gave up
[ "$(grep -c '"pr", "diff"' $d/forge.log)" -eq 2 ] || { echo "expected exactly one retry"; grep -c '"pr", "diff"' $d/forge.log; exit 1; }
