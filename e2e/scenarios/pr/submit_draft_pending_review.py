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
    print(posts)
    b = posts[0]["payload"]
    assert "event" not in b or b["event"] in (None, "PENDING"), b
    assert [c["body"] for c in b["comments"]] == ["draft inline"], b
    tui.quit()
