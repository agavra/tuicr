#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# :submit comment -> confirm modal -> POST payload event=COMMENT with the inline comment; cancel posts nothing.
n=$(e2e_name); sc_pr_open $n
sc_pr_comment $n 'inline for submit'
# cancel first
sc_cmd $n 'submit comment'; e2e_wait_for $n 'Submit review to GitHub'; e2e_expect $n 'Event: Comment'; e2e_expect $n 'Inline: 1'
e2e_keys $n n; e2e_wait_gone $n 'Submit review to GitHub'
[ -z "$(sc_posts $n)" ] || { echo "POST happened after cancel"; sc_posts $n; exit 1; }
# then confirm
sc_cmd $n 'submit comment'; e2e_wait_for $n 'Submit review to GitHub'; e2e_keys $n y
e2e_wait_for $n 'Submitted GitHub review #1001'
posts=$(sc_posts $n); echo "$posts"
[ "$(echo "$posts" | wc -l | tr -d ' ')" = 1 ] || { echo "expected exactly one POST"; exit 1; }
echo "$posts" | python3 -c '
import json,sys
p=json.loads(sys.stdin.readline()); b=p["payload"]
assert p["path"]=="repos/octo/demo/pulls/1/reviews", p["path"]
assert b["event"]=="COMMENT", b
assert b["commit_id"]=="2"*40, b
assert [(c["path"],c["line"],c["side"],c["body"]) for c in b["comments"]]==[("src/greet.rs",1,"RIGHT","inline for submit")], b["comments"]'
sc_quit $n
