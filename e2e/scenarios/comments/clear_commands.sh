#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `:clearc` drops every comment but keeps reviewed marks; `:clear` drops the marks too.
n=$(e2e_name); sc_diff $n
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'first remark'; e2e_keys $n Enter; e2e_wait_for $n 'first remark'
e2e_keys $n g; e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'
sc_cmd $n clearc
e2e_wait_gone $n 'first remark'; e2e_expect $n 'Files · 1/8'; e2e_expect $n '▣ M README\.md'
sc_cursor_on $n 'u32 \{ 2 \}'               # README.md is reviewed, so it is folded: comment elsewhere
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'second remark'; e2e_keys $n Enter; e2e_wait_for $n 'second remark'
sc_cmd $n clear
e2e_wait_gone $n 'second remark'; e2e_wait_for $n 'Files · 0/8'
sc_save $n
! sc_session_has $n 'remark|"reviewed": true' || { echo "cleared state was saved"; sc_session_json $n; exit 1; }
sc_quit $n
