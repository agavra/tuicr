#!/usr/bin/env python3
# :submit approve with a review-level comment as body, via the :submit picker for request-changes too.
from e2e import Scenario, pr_comment, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open()
    pr_comment(tui, "inline for approve")
    tui.keys(";c").wait_for("COMMENT").keys("overall looks good").keys("Enter").wait_for("overall looks good")
    tui.cmd("submit approve").wait_for("Submit review to GitHub").expect("Event: Approve").keys("y")
    tui.wait_for("Submitted GitHub review #1001")
    posts = tui.posts()
    print(posts)
    b = posts[0]["payload"]
    assert b["event"] == "APPROVE", b
    assert "overall looks good" in b["body"], b
    assert [c["body"] for c in b["comments"]] == ["inline for approve"], b
    tui.quit()
