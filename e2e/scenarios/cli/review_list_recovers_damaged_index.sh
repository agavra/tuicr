#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Persistence write-back (p_persist_writeback review): a corrupt or missing `index.json` is rebuilt from sessions/
# instead of reading as empty, so `review list`/`add` keep working and other sessions are not dropped.
n=$(e2e_name); sc_diff $n
repo="$(e2e_dir $n)/repo"
e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'; sc_save $n; sc_quit $n
idx=$(find "$(e2e_dir $n)/data" "$(e2e_dir $n)/home" -name index.json | head -1)
[ -n "$idx" ] || { echo "no index.json written"; exit 1; }
count() { sc_run $n review list --repo "$repo" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))'; }
[ "$(count)" = 1 ] || { echo "expected one session before damage"; exit 1; }
echo '{corrupt' >"$idx"
[ "$(count)" = 1 ] || { echo "corrupt index.json hid the session"; exit 1; }
[ -e "$idx.corrupt" ] || { echo "corrupt index.json was not kept as index.json.corrupt"; exit 1; }
rm "$idx"
[ "$(count)" = 1 ] || { echo "missing index.json hid the session"; exit 1; }
slug=$(sc_run $n review list --repo "$repo" | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["slug"])')
sc_run $n review add --repo "$repo" --session "$slug" "after rebuild" >/dev/null
sc_run $n review comments --repo "$repo" --session "$slug" | grep -q 'after rebuild' || { echo "add after rebuild lost"; exit 1; }
