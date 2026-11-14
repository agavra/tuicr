#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# GitLab happy paths: `:submit comment` posts the summary as an MR note and the inline comment as a discussion with a position;
# `:submit approve` additionally approves; `:submit request-changes` runs the request-changes mutation. Nothing is a draft note.
n=$(e2e_name); sc_forge_open $n https://gitlab.com/octo/demo.git
sc_pr_comment $n 'inline gl note'
e2e_keys $n ';c'; e2e_wait_for $n COMMENT; e2e_keys $n 'gl overall'; e2e_keys $n Enter; e2e_wait_for $n 'gl overall'
sc_cmd $n 'submit comment'; e2e_wait_for $n 'Submit review to GitLab'; e2e_expect $n 'Event: Comment'; e2e_keys $n y
e2e_wait_for $n 'Submitted GitLab review'
posts=$(sc_forge_posts glab)
python3 - "$posts" <<'PY'
import json, sys
calls = [json.loads(l) for l in sys.argv[1].splitlines()]
eps = [c["argv"][1] for c in calls]
assert eps == ["projects/octo%2Fdemo/merge_requests/1/notes", "projects/octo%2Fdemo/merge_requests/1/discussions"], eps
assert calls[0]["payload"] == {"body": "gl overall"}, calls[0]
d = calls[1]["payload"]; pos = d["position"]
assert d["body"] == "inline gl note" and pos["new_path"] == "src/greet.rs" and pos["new_line"] == 1, d
assert pos["position_type"] == "text" and len(pos["head_sha"]) == 40, pos
PY
# the comments are now locked (submitted): approving and requesting changes carry only the vote
sc_cmd $n 'submit approve'; e2e_wait_for $n 'Submit review to GitLab'; e2e_expect $n 'Event: Approve'; e2e_keys $n y
e2e_wait_forge_log 'merge_requests/1/approve'
sc_cmd $n 'submit request-changes'; e2e_wait_for $n 'Submit review to GitLab'; e2e_expect $n 'Event: Request changes'; e2e_keys $n y
e2e_wait_forge_log 'mergeRequestRequestChanges'
e2e_forge_reject 'draft_notes", "--method", "POST"'
sc_quit $n
