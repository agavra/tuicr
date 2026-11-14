#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# :submit draft -> pending review (no event) carrying the inline comment.
n=$(e2e_name); sc_pr_open $n
sc_pr_comment $n 'draft inline'
sc_cmd $n 'submit draft'; e2e_wait_for $n 'Push pending GitHub review'; e2e_expect $n 'Inline: 1'; e2e_keys $n y
e2e_wait_for $n 'Pushed pending GitHub review #1001' 15
posts=$(sc_posts $n); echo "$posts"
echo "$posts" | python3 -c '
import json,sys
p=json.loads(sys.stdin.readline()); b=p["payload"]
assert "event" not in b or b["event"] in (None,"PENDING"), b
assert [c["body"] for c in b["comments"]]==["draft inline"], b
'
sc_quit $n
