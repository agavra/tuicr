#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `tuicr review add` from a second process while the TUI is open: comment appears live; list shows active.
n=$(e2e_name); sc_diff $n
repo="$(e2e_dir $n)/repo"
slug=$(sc_run $n review list --repo "$(e2e_dir $n)/repo" | python3 -c 'import json,sys; l=json.load(sys.stdin); print([s["slug"] for s in l if s.get("active")][0])') \
  || { echo "no active session in review list"; sc_run $n review list --all; exit 1; }
echo "slug=$slug"
sc_run $n review add --repo "$repo" --session "$slug" --target-file README.md --line 4 --side new --type issue "cli injected note" >/dev/null
e2e_wait_for $n 'cli injected note' 15     # picked up by the review watcher (default 1s poll)
sc_run $n review add --repo "$repo" --session "$slug" "cli review-level note" >/dev/null
e2e_wait_for $n 'cli review-level note' 15   # merged live too: it shifts the rows, so wait before moving the cursor
sc_run $n review comments --repo "$repo" --session "$slug" | grep -q 'cli injected note' || { echo "comments lacks injected"; exit 1; }
# TUI-side comment coexists and is not overwritten
sc_cursor_on $n '3   first line'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'tui side note'; e2e_keys $n Enter; e2e_wait_for $n 'tui side note'
sc_save $n
out=$(sc_run $n review comments --repo "$repo" --session "$slug")
for t in 'cli injected note' 'cli review-level note' 'tui side note'; do echo "$out" | grep -q "$t" || { echo "lost: $t"; echo "$out"; exit 1; }; done
sc_quit $n
