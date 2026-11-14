#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `--theme <unknown>` fails at startup (exit 2, lists the bundled themes) without opening the TUI; a valid `--theme`
# beats an unknown `theme` in config.toml.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/config/tuicr; fx_git_repo $d/repo
cd $d/repo
sc_capture $n --no-update-check --theme nosuch-theme
[ "$SC_RC" = 2 ] || { echo "exit status $SC_RC, expected 2"; echo "$SC_OUT"; exit 1; }
grep -q "Unknown theme 'nosuch-theme'" <<<"$SC_OUT" && grep -q 'nord-dark' <<<"$SC_OUT" || { echo "no theme error with the bundled list:"; echo "$SC_OUT"; exit 1; }
printf 'theme = "also-missing"\n' > $d/config/tuicr/config.toml
E2E_CWD=$d/repo e2e_start_keep $n "$TUICR_BIN" --no-update-check --theme nord-dark -r HEAD~2..HEAD
e2e_wait_for $n 'Files · 0/8'
sc_quit $n
