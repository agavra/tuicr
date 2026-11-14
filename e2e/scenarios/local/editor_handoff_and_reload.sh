#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `e` hands the file at the cursor's line to $EDITOR. A terminal editor (vim family) takes the screen and the diff reloads
# when it exits; a windowed one (code) is launched with --goto and the diff only picks the edit up on `:e`.
# The "editors" are stubs that record their arguments and append a line to the file.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/ed; fx_git_repo $d/repo
for ed in vim code; do
  printf '#!/bin/sh\nprintf "%%s\\n" "$*" > "%s/%s.args"\nfor last; do :; done\nf=${last%%:*}\nprintf "edited by %s\\n" >> "$f"\n' "$d/ed" $ed $ed > $d/ed/$ed
  chmod +x $d/ed/$ed
done
open() { # <name> <editor>
  E2E_ENV="EDITOR=$d/ed/$2" E2E_CWD=$d/repo e2e_start "$1" "$TUICR_BIN" --no-update-check -w
  e2e_wait_for "$1" 'unstaged line'; e2e_keys "$1" Escape; sc_cursor_on "$1" 'unstaged line'
}
open $n vim
e2e_keys $n e
e2e_wait_for $n 'edited by vim'                                    # the diff was reloaded after the editor exited
grep -Eq '^\+5 .*/README\.md$' $d/ed/vim.args || { echo "vim got: $(cat $d/ed/vim.args)"; exit 1; }
sc_quit $n
m=${n}_windowed; open $m code
e2e_keys $m e
e2e_wait_file $d/ed/code.args
grep -Eq -- '^--goto .*/README\.md:5$' $d/ed/code.args || { echo "code got: $(cat $d/ed/code.args)"; exit 1; }
e2e_reject $m 'edited by code' 1                                   # a windowed editor does not trigger a reload
sc_cmd $m e; e2e_wait_for $m 'edited by code'                       # `:e` does
sc_quit $m
