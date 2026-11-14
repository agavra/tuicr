#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# #868: submit maps comments against the displayed (selected-range) diff. src/greet.rs exists
# only in the newest commit's diff, not in the cumulative one (variant subsetonly); a comment on
# it must go out inline, not be sent to the "unmappable" resolver.
n=$(e2e_name); sc_pr_open $n subsetonly
e2e_keys $n ';k'; e2e_keys $n Enter                  # narrow to the newest commit (2222222)
e2e_wait_for $n 'Commit Message \(2222222\)'; e2e_wait_for $n 'greet\.rs'
e2e_keys $n ';l'; e2e_settle $n
sc_pr_comment $n 'inline on subset-only file'
sc_cmd $n 'submit comment'
e2e_wait_for $n 'Submit review to GitHub|nmappable|Resolve' 
e2e_expect $n 'Submit review to GitHub'; e2e_expect $n 'Inline: 1'
e2e_keys $n y
e2e_wait_for $n 'Submitted GitHub review #1001'
sc_posts $n | python3 -c '
import json,sys
p=json.loads(sys.stdin.readline())["payload"]
got=[(c["path"],c["line"],c["body"]) for c in p["comments"]]
assert got==[("src/greet.rs",1,"inline on subset-only file")], got'
sc_quit $n
