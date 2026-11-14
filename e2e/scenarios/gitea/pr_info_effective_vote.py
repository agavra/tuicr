#!/usr/bin/env python3
# Covers #914: the PR description panel's review summary shows each reviewer's *effective* vote. alice APPROVED
# then left a later plain COMMENT: Gitea keeps her approval; bob's REQUEST_CHANGES was dismissed so it no longer counts.
# Baseline let the newer COMMENT replace the approval (alice listed as Commented) and counted bob's dismissed vote.
import json
from typing import Any

from e2e import Scenario, scenario


def review(i: int, user: str, state: str, at: str, **kw: Any) -> dict[str, Any]:
    base: dict[str, Any] = {
        "id": i,
        "state": state,
        "body": "",
        "user": {"login": user},
        "submitted_at": at,
        "comments_count": 0,
        "commit_id": "",
        "html_url": f"u{i}",
    }
    return dict(base, **kw)


@scenario
def main(s: Scenario) -> None:
    s.dir.mkdir(parents=True, exist_ok=True)
    reviews = [
        review(1, "alice", "APPROVED", "2026-01-02T05:00:00Z"),
        review(2, "alice", "COMMENT", "2026-01-02T06:00:00Z"),
        review(3, "bob", "REQUEST_CHANGES", "2026-01-02T05:30:00Z", dismissed=True),
    ]
    (s.dir / "gt_reviews.json").write_text(json.dumps(reviews))
    tui = s.forge_open("https://gitea.example.com/octo/demo.git")
    tui.expect("Approved: @alice")  # else: alice's approval was lost to a later COMMENT
    tui.reject("Changes requested: @bob|Requested changes: @bob", 1)
    tui.reject("Commented: @alice", 1)
    tui.quit()
