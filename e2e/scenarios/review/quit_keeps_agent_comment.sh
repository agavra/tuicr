#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `:q` with only an unsaved reviewed mark discards the session, but not one holding a comment an agent added since the
# last poll (polling is off here, so the TUI never saw it): it refuses and shows the comment instead. Header: panel G6-1.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/config/tuicr; fx_git_repo $d/repo
printf 'review_watch_interval_ms = 0\n' > $d/config/tuicr/config.toml
E2E_CWD=$d/repo e2e_start_keep $n "$TUICR_BIN" --no-update-check -r HEAD~2..HEAD
e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
repo=$d/repo
slug=$(sc_run $n review list --repo "$repo" | python3 -c 'import json,sys; l=json.load(sys.stdin); print([s["slug"] for s in l if s.get("active")][0])')
sc_run $n review add --repo "$repo" --session "$slug" --target-file README.md --line 4 "agent note" >/dev/null
e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'
e2e_keys $n ":q" Enter
e2e_wait_for $n 'Saved session has comments'
sc_session_has $n 'agent note' || { echo "quit deleted the agent's comment"; exit 1; }
sc_quit $n
