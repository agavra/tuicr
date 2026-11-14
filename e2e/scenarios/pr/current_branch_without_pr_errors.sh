#!/usr/bin/env bash
# `tuicr pr` on a branch no open PR uses fails before the TUI, pointing at `tuicr pr <number>`.
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
n=$(e2e_name); d=$(e2e_dir "$n"); mkdir -p "$d"
fx_pr_repo "$d/repo"
git -C "$d/repo" checkout -q -b unrelated-branch
cd "$d/repo"
sc_capture "$n" --no-update-check pr
[ "$SC_RC" = 1 ] || { echo "expected exit 1, got $SC_RC"; echo "$SC_OUT"; exit 1; }
grep -q 'No open pull request found for branch `unrelated-branch`' <<<"$SC_OUT" || { echo "no branch diagnostic:"; echo "$SC_OUT"; exit 1; }
grep -q 'tuicr pr <number>' <<<"$SC_OUT" || { echo "no hint:"; echo "$SC_OUT"; exit 1; }
