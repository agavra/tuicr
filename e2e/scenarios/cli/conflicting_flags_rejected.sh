#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# option conflicts are checked on the merged root/tui/pr flags,
# and `pr` rejects local-review flags. Each invocation must exit with a conflict
# error instead of opening the TUI.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
for args in "-A tui --file README.md" "--file README.md tui --path src/" "-r HEAD tui -A" "-w tui --file README.md" "pr 1 --file README.md" "pr 1 -A" "-w pr 1"; do
  cd $d/repo; sc_capture $n --no-update-check $args
  [ "$SC_RC" -ne 0 ] || { echo "FAIL: 'tuicr $args' exited 0 instead of rejecting the flags. Output: $SC_OUT"; exit 1; }
  echo "$SC_OUT" | grep -Eq 'cannot be|conflict' || { echo "FAIL: 'tuicr $args' not rejected with a conflict error. Output: $SC_OUT"; exit 1; }
  echo "ok: tuicr $args rejected"
done
# still allowed: -r with -w (split across levels)
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD tui -w
e2e_wait_for $n 'Files ·'; sc_quit $n
