#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #896: switching from a commit-range review to "Staged changes" through :commits
#  (a) clears the range's commit pane (baseline kept it), and
#  (b) saves the outgoing range session (its reviewed mark) before replacing it; baseline lost it.
n=$(e2e_name)
sc_diff $n
e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'
sc_cmd $n commits; e2e_wait_for $n 'Staged changes'
e2e_keys $n j; e2e_keys $n Enter
e2e_wait_for $n 'staged\.txt'; e2e_settle $n 0.4
e2e_reject $n '┌ Commits' || { echo "stale commit pane kept after switching to a local target"; exit 1; }
sc_quit $n
saved=$(sc_session_json $n | python3 -c '
import sys,json,re
txt=sys.stdin.read()
dec=json.JSONDecoder(); i=0; n=0
while i<len(txt):
    while i<len(txt) and txt[i].isspace(): i+=1
    if i>=len(txt): break
    o,i=dec.raw_decode(txt,i)
    if o.get("diff_source")=="commit_range" and any(f.get("reviewed") for f in o["files"].values()): n+=1
print(n)')
echo "range sessions with a reviewed file on disk: $saved"
[ "$saved" -ge 1 ] || { echo "outgoing range session (reviewed mark) was not saved on the target switch"; exit 1; }
