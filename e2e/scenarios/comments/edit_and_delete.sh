#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Edit (i) and delete (dd) a comment at the cursor.
n=$(e2e_name); sc_diff $n
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n 'COMMENT'; e2e_keys $n 'original text'; e2e_keys $n Enter
e2e_wait_for $n 'original text'
# m jumps to the next comment (the cursor lands on the comment box)
e2e_keys $n g; e2e_keys $n m
e2e_keys $n i; e2e_wait_for $n 'COMMENT'
e2e_keys $n ' EDITED'; e2e_keys $n Enter
e2e_wait_for $n 'original text EDITED'
# delete
e2e_keys $n d d
e2e_wait_gone $n 'original text EDITED'
sc_save $n
[ -n "$(sc_session_json $n)" ] || { echo "no saved session to check"; exit 1; }
if sc_session_json $n | grep -q 'original text'; then echo "deleted comment still in session json"; sc_session_json $n; exit 1; fi
sc_quit $n
