#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #897: a Gerrit change URL copied from the browser (patch-set path, ?query, #message anchor) opens the change.
# Baseline rejects anything after the change number.
n=$(e2e_name)
export E2E_GERRIT_HOST=127.0.0.1   # origin must name the same host as the URL
for t in 'http://127.0.0.1:@PORT@/c/proj/+/1/1?tab=comments#message-abc' 'http://127.0.0.1:@PORT@/c/proj/+/1#message-abc'; do
  E2E_GERRIT_TARGET="$t" sc_gerrit_open $n root
  e2e_wait_for $n 'a\.txt' 20 || { echo "URL with patch set, query and fragment did not open the change"; exit 1; }
  e2e_stop $n; sc_gerrit_stop $n
done
