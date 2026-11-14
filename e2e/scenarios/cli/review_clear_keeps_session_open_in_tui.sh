#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `tuicr review clear` empties a session but leaves the file alone while a running TUI owns it, so an agent can keep
# adding comments the TUI picks up (an inactive emptied session is removed: cli/review_json_input_delete_clear).
n=$(e2e_name); sc_diff $n
repo="$(e2e_dir $n)/repo"
slug=$(sc_run $n review list --repo "$repo" | python3 -c 'import json,sys; l=json.load(sys.stdin); print([s["slug"] for s in l if s.get("active")][0])')
sc_run $n review add --repo "$repo" --session "$slug" --target-file README.md --line 4 "first agent note" >/dev/null
e2e_wait_for $n 'first agent note' 15
sc_run $n review clear --repo "$repo" --session "$slug" >/dev/null
e2e_wait_gone $n 'first agent note' 15
[ -n "$(sc_session_json $n)" ] || { echo "clear deleted the session a live TUI owns"; exit 1; }
sc_run $n review add --repo "$repo" --session "$slug" --target-file README.md --line 4 "second agent note" >/dev/null
e2e_wait_for $n 'second agent note' 15
sc_quit $n
