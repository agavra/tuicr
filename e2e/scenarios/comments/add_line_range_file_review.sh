#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Add line / range / file / review comments; they render inline and persist with :w.
n=$(e2e_name); sc_diff $n
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n 'COMMENT'
e2e_keys $n 'line note alpha'; e2e_keys $n Enter
e2e_wait_for $n 'line note alpha'; e2e_expect $n '── L4 '
# range comment over two rows (gone.txt deleted lines) via visual mode
sc_cursor_on $n '1 ▌ delete me'
e2e_keys $n v j c; e2e_wait_for $n 'COMMENT'
e2e_keys $n 'range note beta'; e2e_keys $n Enter
e2e_wait_for $n 'range note beta'; e2e_expect $n 'L1-L2|L1.{0,3}2'
# file comment
e2e_keys $n g; e2e_keys $n C; e2e_wait_for $n 'COMMENT'
e2e_keys $n 'file note gamma'; e2e_keys $n Enter
e2e_wait_for $n 'file note gamma'
# review-level comment
e2e_keys $n ';c'; e2e_wait_for $n 'COMMENT'
e2e_keys $n 'review note delta'; e2e_keys $n Enter
e2e_wait_for $n 'review note delta'
# comment navigator sidebar lists them
e2e_expect $n 'Comments'
# persist, then verify on disk
sc_save $n
j=$(sc_session_json $n)
for t in 'line note alpha' 'range note beta' 'file note gamma' 'review note delta'; do
  echo "$j" | grep -q "$t" || { echo "session json lacks '$t'"; echo "$j" | head -c 3000; exit 1; }
done
sc_quit $n
