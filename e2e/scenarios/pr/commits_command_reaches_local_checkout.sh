#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #916: from `tuicr pr 1` in a real checkout, `:commits` must reach the checkout's local targets
# (Unstaged changes, ...) and loading one must work. Baseline had swapped in the PR no-op VCS: `:commits` answered
# "No commits or ..." and the selector never opened.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo; _fx_git $d/repo remote add origin https://github.com/octo/demo.git
E2E_ENV="E2E_VARIANT=rich" E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check pr 1
e2e_wait_for $n 'PR Description' 20
sc_cmd $n commits
e2e_expect $n 'Unstaged changes' || { echo ":commits in PR mode did not open the local target selector"; exit 1; }
e2e_keys $n Enter
e2e_expect $n 'unstaged line' || { echo "local Unstaged changes did not load after leaving PR mode"; exit 1; }
e2e_reject $n 'PR Mode' 1
sc_quit $n
