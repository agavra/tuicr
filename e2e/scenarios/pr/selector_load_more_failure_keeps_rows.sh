#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #908: a failed load-more on the Pull Requests tab must keep the already loaded rows (and the
# load-more row for a retry). Baseline replaced the whole list with the error panel. 45 rows, page size 30; fake gh fails any
# `--limit` above 31 (the load-more request asks for 61).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_pr_repo $d/repo
E2E_ENV="E2E_LIST_ROWS=45 E2E_LIST_FAIL_ABOVE=31" E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check
e2e_wait_for $n 'Pull Requests'; e2e_settle $n; e2e_keys $n Tab; e2e_wait_for $n 'Row 1 '
for i in $(seq 30); do e2e_keys $n j; done; e2e_settle $n
e2e_screen $n | grep -E '▶|load more' | cut -c1-110
e2e_keys $n Enter
# barrier: the failing load-more request (--limit 61) reached the forge; then the UI absorbs the failure
e2e_wait_forge_log '"--limit", "61"'; e2e_settle $n 0.4
e2e_screen $n | sed -n '1,8p;40,45p' | cut -c1-140
e2e_expect $n 'Row 1 ' 2 || { echo "loaded rows vanished after a failed load-more"; exit 1; }
e2e_expect $n 'load more pull requests' 2 || { echo "load-more row gone, cannot retry"; exit 1; }
sc_quit $n
