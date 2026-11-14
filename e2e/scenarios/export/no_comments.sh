#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `:clip` on a review with no comments: tuicr says so instead of exporting an empty document; the clipboard stays untouched.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
sc_fake_clipboard $n
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
sc_cmd $n clip
e2e_wait_for $n 'No comments to export'
e2e_alive $n || { echo "tuicr exited after :clip"; exit 1; }
[ ! -e $d/clipboard.txt ] || { echo "clipboard written for an empty review:"; cat $d/clipboard.txt; exit 1; }
sc_quit $n
