#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #914: the PR description panel's review summary shows each reviewer's *effective* vote. alice APPROVED
# then left a later plain COMMENT: Gitea keeps her approval; bob's REQUEST_CHANGES was dismissed so it no longer counts.
# Baseline let the newer COMMENT replace the approval (alice listed as Commented) and counted bob's dismissed vote.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d
python3 - $d <<'PY'
import json, sys
def rv(i, user, state, at, **kw):
    return dict({"id": i, "state": state, "body": "", "user": {"login": user}, "submitted_at": at, "comments_count": 0, "commit_id": "", "html_url": "u%d" % i}, **kw)
json.dump([rv(1, "alice", "APPROVED", "2026-01-02T05:00:00Z"), rv(2, "alice", "COMMENT", "2026-01-02T06:00:00Z"),
           rv(3, "bob", "REQUEST_CHANGES", "2026-01-02T05:30:00Z", dismissed=True)], open(sys.argv[1] + "/gt_reviews.json", "w"))
PY
sc_forge_open $n https://gitea.example.com/octo/demo.git
e2e_expect $n 'Approved: @alice' || { echo "alice's approval was lost to a later COMMENT"; exit 1; }
e2e_reject $n 'Changes requested: @bob|Requested changes: @bob' 1
e2e_reject $n 'Commented: @alice' 1
sc_quit $n
