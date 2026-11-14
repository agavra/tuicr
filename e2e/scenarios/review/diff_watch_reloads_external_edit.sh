#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `diff_watch_interval_ms` re-reads the working-tree diff on a timer: an edit made outside shows up without `:e`.
# Without the setting (default 0) the same edit stays invisible until `:e`.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/config/tuicr; fx_git_repo $d/repo
printf 'diff_watch_interval_ms = 200\n' > $d/config/tuicr/config.toml
E2E_CWD=$d/repo e2e_start_keep $n "$TUICR_BIN" --no-update-check -w
e2e_wait_for $n 'unstaged line'
printf 'edited behind its back\n' >> $d/repo/README.md
e2e_wait_for $n 'edited behind its back' 15
sc_quit $n
m=${n}_off; git -C $d/repo checkout -q README.md; printf 'x\n' >> $d/repo/README.md
E2E_CWD=$d/repo e2e_start $m "$TUICR_BIN" --no-update-check -w
e2e_wait_for $m '5 ▌ x'
printf 'edited behind its back\n' >> $d/repo/README.md
e2e_reject $m 'edited behind its back' 2
sc_cmd $m e; e2e_wait_for $m 'edited behind its back'
sc_quit $m
