#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `ZZ` saves the unsaved reviewed mark, copies the review and quits; `ZQ` quits without copying and without saving it.
for k in ZZ ZQ; do
  n=$(e2e_name)_$k; d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
  sc_fake_clipboard $n
  E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
  e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
  sc_cursor_on $n '4 ▌ second line'
  e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n "note for $k"; e2e_keys $n Enter; e2e_wait_for $n "note for $k"
  e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'   # the comment autosaved; the mark is the unsaved part
  e2e_keys $n $k
  e2e_wait_exit $n
  if [ $k = ZZ ]; then
    sc_session_has $n '"reviewed": true' || { echo "ZZ did not save the reviewed mark"; exit 1; }
    clip=$(sc_clipboard $n); grep -q 'note for ZZ' <<<"$clip" || { echo "ZZ did not copy the review"; exit 1; }
  else
    ! sc_session_has $n '"reviewed": true' || { echo "ZQ saved the reviewed mark"; exit 1; }
    [ ! -e $d/clipboard.txt ] || { echo "ZQ copied the review"; exit 1; }
  fi
done
