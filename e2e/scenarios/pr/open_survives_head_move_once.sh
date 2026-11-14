#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #893: head moves once while PR #1 opens (fake gh: head 2222.. until the first data call, 4444.. after).
# Fixed: the open re-reads the head, sees it changed, retries once and settles on 4444.. (session keyed to it).
# Baseline: opens silently pairing the old head's metadata with the newer diff, session keyed to 2222...
n=$(e2e_name)
E2E_ENV="E2E_HEAD_MODE=once" sc_pr_open $n
sc_pr_comment $n SNAPSHOT-DRAFT
sc_save $n
j=$(sc_session_json $n)
echo "$j" | grep -o '"head_sha": *"[0-9a-f]*"' | sort -u
echo "$j" | grep -q '"head_sha": *"4444444444444444444444444444444444444444"' || { echo "session not keyed to the settled head"; exit 1; }
! echo "$j" | grep -q '"head_sha": *"2222222222222222222222222222222222222222"' || { echo "session pinned to the stale head"; exit 1; }
sc_quit $n
