#!/usr/bin/env python3
# :submit draft -> pending review (no event) carrying the inline comment.
from e2e import Scenario, pr_comment, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open()
    pr_comment(tui, "draft inline")
    tui.cmd("submit draft").wait_for("Push pending GitHub review").expect("Inline: 1").keys("y")
    tui.wait_for("Pushed pending GitHub review #1001", 15)
    posts = tui.posts()
    assert len(posts) == 1, f"expected exactly one review POST: {posts}"
    assert posts[0]["path"] == "repos/octo/demo/pulls/1/reviews", posts[0]["path"]
    b = posts[0]["payload"]
    # a pending review is one created WITHOUT an event (GitHub rejects event "PENDING", and any other event publishes)
    assert "event" not in b, f"the draft review carries an event: {b}"
    assert [c["body"] for c in b["comments"]] == ["draft inline"], b
    tui.quit()
