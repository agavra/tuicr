#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Gitea happy paths: `:submit comment` / `approve` / `request-changes` each POST one review with Gitea's own event names
# (COMMENT, APPROVED, REQUEST_CHANGES), the inline comment as a single-line `new_position`; request-changes carries a summary.
n=$(e2e_name); sc_forge_open $n https://gitea.example.com/octo/demo.git
sc_pr_comment $n 'inline gt note'
e2e_keys $n ';c'; e2e_wait_for $n 'ctrl-s save'; e2e_keys $n 'gt overall'; e2e_keys $n Enter; e2e_wait_for $n 'Review comment added'
sc_cmd $n 'submit comment'; e2e_wait_for $n 'Submit review to Gitea'; e2e_expect $n 'Event: Comment'; e2e_keys $n y
e2e_wait_for $n 'Submitted Gitea review'
sc_cmd $n 'submit approve'; e2e_wait_for $n 'Submit review to Gitea'; e2e_expect $n 'Event: Approve'; e2e_keys $n y
e2e_wait_forge_count '"POST".*pulls/1/reviews"' 2 15
e2e_keys $n ';c'; e2e_wait_for $n 'ctrl-s save'; e2e_keys $n 'gt please fix'; e2e_keys $n Enter; e2e_wait_for $n 'Review comment added'
sc_cmd $n 'submit request-changes'; e2e_wait_for $n 'Submit review to Gitea'; e2e_expect $n 'Event: Request changes'; e2e_keys $n y
e2e_wait_forge_count '"POST".*pulls/1/reviews"' 3 15
posts=$(sc_forge_posts tea)
python3 - "$posts" <<'PY'
import json, sys
calls = [json.loads(l) for l in sys.argv[1].splitlines()]
pl = [c["payload"] for c in calls]
assert [p["event"] for p in pl] == ["COMMENT", "APPROVED", "REQUEST_CHANGES"], [p.get("event") for p in pl]
assert all(c["argv"][-1] == "/repos/octo/demo/pulls/1/reviews" for c in calls), [c["argv"] for c in calls]
assert pl[0]["body"] == "gt overall" and pl[0]["comments"] == [{"path": "src/greet.rs", "body": "inline gt note", "new_position": 1}], pl[0]
assert pl[1]["comments"] == [] and pl[2]["body"] == "gt please fix", pl
PY
sc_quit $n
