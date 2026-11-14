#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `-A` (--all-files) puts every tracked file in the review, including ones with no change.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -A
e2e_wait_for $n 'Files · 0/7'                                  # 7 tracked files, no status letters: nothing is a change
e2e_expect $n '▢ README\.md'; e2e_expect $n '▢ +main\.rs'; e2e_expect $n '▢ staged\.txt'; e2e_expect $n '\+0 -0'
sc_quit $n
