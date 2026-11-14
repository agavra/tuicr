#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# A repo-root `.tuicrignore` (gitignore syntax) drops matching files from the review at load time: patterns, a directory
# rule and a `!` re-include; a later edit is picked up by `:e`.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
printf '*.bin\nnotes_*.txt\nsrc/\n!src/main.rs\n' > $d/repo/.tuicrignore
E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
e2e_wait_for $n 'Files · 0/5'                     # 8 files - data.bin - notes_cjk.txt - src/lib.rs
e2e_keys $n Escape
e2e_expect $n 'M README\.md'; e2e_expect $n 'tabs\.txt'; e2e_expect $n 'main\.rs'
e2e_reject $n 'data\.bin|notes_cjk|lib\.rs'
printf '*.bin\n' > $d/repo/.tuicrignore
sc_cmd $n e; e2e_wait_for $n 'Files · 0/7'; e2e_expect $n 'notes_cjk\.txt'; e2e_expect $n 'lib\.rs'; e2e_reject $n 'data\.bin'
sc_quit $n
