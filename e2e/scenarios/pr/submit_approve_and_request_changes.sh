#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# :submit approve with a review-level comment as body, via the :submit picker for request-changes too.
n=$(e2e_name); sc_pr_open $n
sc_pr_comment $n 'inline for approve'
e2e_keys $n ';c'; e2e_wait_for $n COMMENT; e2e_keys $n 'overall looks good'; e2e_keys $n Enter; e2e_wait_for $n 'overall looks good'
sc_cmd $n 'submit approve'; e2e_wait_for $n 'Submit review to GitHub'; e2e_expect $n 'Event: Approve'; e2e_keys $n y
e2e_wait_for $n 'Submitted GitHub review #1001'
posts=$(sc_posts $n); echo "$posts"
echo "$posts" | python3 -c '
import json,sys
p=json.loads(sys.stdin.readline()); b=p["payload"]
assert b["event"]=="APPROVE", b
assert "overall looks good" in b["body"], b
assert [c["body"] for c in b["comments"]]==["inline for approve"], b
'
sc_quit $n
