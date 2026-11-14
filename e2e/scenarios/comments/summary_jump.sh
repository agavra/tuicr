#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# :summary lists pending local comments; Enter jumps to one; Esc returns.
n=$(e2e_name); sc_diff $n
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'summary one'; e2e_keys $n Enter; e2e_wait_for $n 'summary one'
e2e_keys $n G; e2e_keys $n C; e2e_wait_for $n COMMENT; e2e_keys $n 'summary two'; e2e_keys $n Enter; e2e_wait_for $n 'summary two'
sc_cmd $n summary
e2e_wait_for $n 'SUMMARY'; e2e_expect $n 'summary one'; e2e_expect $n 'summary two'
e2e_keys $n j Enter
e2e_wait_gone $n 'SUMMARY'
e2e_expect $n 'summary (one|two)'
sc_cmd $n summary; e2e_wait_for $n 'SUMMARY'
e2e_keys $n Escape; e2e_wait_gone $n 'SUMMARY'
sc_quit $n
