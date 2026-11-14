#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #910: a GitLab MR URL with an upper-case host, a port and userinfo, in a subgroup, must reach glab with the bare host.
# Baseline kept userinfo and port in the host it handed to glab (`--hostname me@GitLab.Example.com:8443`), which no glab
# configuration matches; the fix passes the bare hostname.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/home; rm -rf $d/repo; mkdir -p $d/repo; git init -q $d/repo
cd $d/repo
export E2E_FORGE_LOG=$d/forge.log; : > $E2E_FORGE_LOG
# The fake glab has no fixture for this project, so the open fails: only the host handed to glab is asserted.
e2e_allow_unexpected_forge_calls
sc_capture $n --no-update-check pr 'https://me@GitLab.Example.com:8443/grp/sub/proj/-/merge_requests/7'
echo "$SC_OUT" | head -5
[ "$SC_RC" -ne 0 ] || { echo "tuicr pr unexpectedly succeeded against a forge without fixtures"; exit 1; }
echo "--- forge calls:"; python3 -c "
import json
for l in open('$E2E_FORGE_LOG'): j=json.loads(l); print(j['tool'], ' '.join(j['argv'])[:120])"
grep -q '"tool": "glab"' $E2E_FORGE_LOG || { echo "glab was never called"; exit 1; }
# the host handed to glab must be the bare hostname: baseline passed `me@GitLab.Example.com:8443` (userinfo + port)
if grep -E '"--hostname", "[^"]*[@:]' $E2E_FORGE_LOG >/dev/null; then echo "glab was called with userinfo/port in --hostname"; exit 1; fi
grep -q '"--hostname", "GitLab.Example.com"' $E2E_FORGE_LOG || { echo "no glab call with the bare host"; exit 1; }
