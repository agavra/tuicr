#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Bitbucket Cloud happy paths: a submit is a series of plain comments (summary, then each inline one with its `inline` anchor,
# never `pending`); approve is `bkt pr approve`, request-changes the `/request-changes` endpoint.
n=$(e2e_name); sc_forge_open $n https://bitbucket.org/octo/demo.git
sc_pr_comment $n 'inline bb note'
e2e_keys $n ';c'; e2e_wait_for $n COMMENT; e2e_keys $n 'bb overall'; e2e_keys $n Enter; e2e_wait_for $n 'bb overall'
sc_cmd $n 'submit comment'; e2e_wait_for $n 'Submit review to Bitbucket'; e2e_expect $n 'Event: Comment'; e2e_keys $n y
e2e_wait_for $n 'Submitted Bitbucket review'
python3 - "$(sc_forge_posts bkt)" <<'PY'
import json, sys
calls = [json.loads(l) for l in sys.argv[1].splitlines()]
assert [c["argv"][1] for c in calls] == ["/2.0/repositories/octo/demo/pullrequests/1/comments"] * 2, [c["argv"] for c in calls]
assert calls[0]["payload"] == {"content": {"raw": "bb overall"}}, calls[0]
assert calls[1]["payload"] == {"content": {"raw": "inline bb note"}, "inline": {"path": "src/greet.rs", "to": 1}}, calls[1]
PY
sc_cmd $n 'submit approve'; e2e_wait_for $n 'Submit review to Bitbucket'; e2e_expect $n 'Event: Approve'; e2e_keys $n y
e2e_wait_forge_log '"pr", "approve", "1"'
sc_cmd $n 'submit request-changes'; e2e_wait_for $n 'Submit review to Bitbucket'; e2e_expect $n 'Event: Request changes'; e2e_keys $n y
e2e_wait_forge_log 'pullrequests/1/request-changes'
sc_quit $n
