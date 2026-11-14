#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `:wq` saves the session and asks whether to copy the review; `n` quits without copying, `y` copies it then quits.
for answer in n y; do
  n=$(e2e_name)_$answer; d=$(e2e_dir $n); mkdir -p $d; fx_git_repo $d/repo
  sc_fake_clipboard $n
  E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
  e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
  sc_cursor_on $n '4 ▌ second line'
  e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n "note for $answer"; e2e_keys $n Enter; e2e_wait_for $n "note for $answer"
  sc_cmd $n wq
  e2e_wait_for $n 'Copy review to clipboard\?'
  e2e_alive $n || { echo ":wq quit before the prompt was answered"; exit 1; }
  sc_session_has $n "note for $answer" || { echo "session was not saved by :wq"; exit 1; }
  e2e_keys $n $answer
  e2e_wait_exit $n
  if [ $answer = y ]; then clip=$(sc_clipboard $n); grep -q 'note for y' <<<"$clip" || { echo "review not copied"; exit 1; }
  else [ ! -e $d/clipboard.txt ] || { echo "answering n still copied"; exit 1; }; fi
done
