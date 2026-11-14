#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #886: `review list --repo forge:<host>/<owner…>/<repo>` (the path tuicr stores for PR
# sessions) keeps every owner segment. For a Gerrit project `platform/build/tools` on a host the selector parser
# cannot tell is Gerrit (no "gerrit" in the name), the generic last-two-segments rule used to give owner `build`, so
# the selector never matched the saved PR session. Also: an `owner/.git` repo segment is not a selector.
n=$(e2e_name)
E2E_GERRIT_HOST=review.example.test E2E_GERRIT_PROJECT=platform/build/tools sc_gerrit_open $n root
e2e_wait_for $n 'a.txt' 20
e2e_settle $n; e2e_keys $n '}'; sc_cursor_on $n 'line two'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'identity note'; e2e_keys $n Enter; e2e_wait_for $n 'identity note'
sc_save $n
echo "--- all sessions"; sc_run $n review list --all | tee "$(e2e_dir $n)/all.json"
slug=$(python3 -c 'import json,sys; l=json.load(open(sys.argv[1])); print(l[0]["slug"])' "$(e2e_dir $n)/all.json")
echo "slug=$slug"
host=review.example.test
out=$(sc_run $n review list --repo "forge:$host/platform/build/tools" 2>&1)
echo "--- forge:$host/platform/build/tools"; echo "$out"
echo "$out" | grep -q "$slug" || { echo "forge:$host/platform/build/tools did not select the saved Gerrit PR session"; exit 1; }
bad=$(sc_run $n review list --repo "forge:$host/platform/build/.git" 2>&1) || true   # may be an error: only "no session selected" matters
echo "--- forge:$host/platform/build/.git"; echo "$bad"
! echo "$bad" | grep -q "$slug" || { echo "owner/.git selector matched a session"; exit 1; }
sc_quit $n
