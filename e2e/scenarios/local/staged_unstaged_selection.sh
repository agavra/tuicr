#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Selection rows: "Unstaged changes" / "Staged changes", picking one, and both with Space.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Unstaged changes'; e2e_expect $n 'Staged changes'; e2e_expect $n 'fix: greeting'
# staged only
e2e_keys $n j Enter
e2e_expect $n '▢ A staged\.txt'; e2e_expect $n 'staged addition'; e2e_reject $n 'README\.md|untracked'
# back to selector, unstaged only
e2e_keys $n Escape; e2e_keys $n ':commits' Enter; e2e_wait_for $n 'Unstaged changes'
e2e_keys $n Enter
e2e_expect $n '▢ M README\.md'; e2e_expect $n 'unstaged line'; e2e_expect $n '▢ A untracked\.txt'; e2e_reject $n 'staged addition'
# both rows (Space on each) -> union
e2e_keys $n Escape; e2e_keys $n ':commits' Enter; e2e_wait_for $n 'Unstaged changes'
e2e_keys $n Space j Space Enter
e2e_expect $n '▢ A staged\.txt'; e2e_expect $n '▢ M README\.md'
sc_quit $n
