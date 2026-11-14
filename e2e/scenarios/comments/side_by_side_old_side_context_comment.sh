#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #887: a context row exists on both sides of side-by-side; an old-side (left pane) comment
# on it must show its input box while typing and render after saving. Before, only New-side comments were handled
# on context rows: the input was invisible and the stored comment vanished.
n=$(e2e_name); sc_diff $n
sc_cmd $n diff
e2e_wait_for $n '3  first line +│ +3  first line'
sc_cursor_on $n '3  first line'
e2e_keys $n ';h'; e2e_settle $n
e2e_keys $n c; e2e_wait_for $n COMMENT
e2e_keys $n 'oldside note xyz'
e2e_wait_for $n 'oldside note xyz' || { echo "old-side comment input box not visible"; exit 1; }
e2e_keys $n Enter; e2e_settle $n
e2e_wait_for $n 'oldside note xyz' || { echo "saved old-side comment not rendered on context row"; exit 1; }
sc_session_json $n | grep -q 'oldside note xyz' || { echo "comment not stored"; exit 1; }
sc_session_json $n | grep -q '"side": *"old"' || { echo "comment not stored on the old side"; sc_session_json $n | head -40; exit 1; }
sc_quit $n
