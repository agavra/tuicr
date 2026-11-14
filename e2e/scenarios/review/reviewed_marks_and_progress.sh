#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# r toggles file reviewed, R toggles hunk reviewed; progress counter and persistence.
n=$(e2e_name); sc_diff $n
e2e_expect $n 'Files · 0/8'
e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'; e2e_expect $n '✓ M README\.md|✓.*README\.md'
e2e_keys $n r; e2e_wait_for $n 'Files · 0/8'
e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'
# hunk reviewed: go to src/main.rs hunk and press R
e2e_keys $n G; e2e_keys $n R
e2e_expect $n 'reviewed|✓'
sc_save $n
sc_session_json $n | grep -q '"reviewed": *true' || { echo "no reviewed=true in session"; sc_session_json $n | head -c 2000; exit 1; }
sc_session_json $n | tr -d ' \n' | grep -Eq '"reviewed_hunks":\["' || { echo "no reviewed_hunks entry"; sc_session_json $n | head -c 2000; exit 1; }
sc_quit $n
