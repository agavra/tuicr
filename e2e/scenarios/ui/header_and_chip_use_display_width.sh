#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #906: CJK in the header title and in a commit row's branch chip are measured by display width.
# Baseline: chip `[ブランチ]` is cut by byte count (`[ブラ...]`) and its row overflows the box (`· just n│`); the header measures
# by chars so the repo@branch slug is clipped early.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; r="$d/日本語リポジトリ"; fx_git_repo "$r"
git -C "$r" branch '機能/日本語ブランチ名前長い'; git -C "$r" checkout -q '機能/日本語ブランチ名前長い'
git -C "$r" branch 'ブランチ' HEAD~1
E2E_COLS=100 E2E_CWD="$r" e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
e2e_wait_for $n 'Commits'; e2e_settle $n 0.4
e2e_screen $n | sed -n 4p | grep -q '\[ブランチ\]' || { echo "branch chip truncated by bytes"; exit 1; }
e2e_screen $n | sed -n 4p | grep -q 'just now│' || { echo "commit row overflows its box"; exit 1; }
e2e_screen $n | sed -n 1p | grep -q 'ブランチ名前長い/c' || { echo "header title measured by chars: slug clipped early"; exit 1; }
sc_quit $n
