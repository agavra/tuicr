#!/usr/bin/env python3
# :submit approve with a review-level comment as body, via the :submit picker for request-changes too.
from e2e import Scenario, pr_comment, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open()
    pr_comment(tui, "inline for approve")
    tui.comment("overall looks good", key=";c")
    tui.cmd("submit approve").wait_for("Submit review to GitHub").expect("Event: Approve").keys("y")
    tui.wait_for("Submitted GitHub review #1001")
    posts = tui.posts()
    assert len(posts) == 1, f"expected exactly one review POST for approve: {posts}"
    b = posts[0]["payload"]
    assert b["event"] == "APPROVE", b
    assert "overall looks good" in b["body"], b
    assert [c["body"] for c in b["comments"]] == ["inline for approve"], b
    # request changes through the bare :submit picker (Comment / Approve / Request changes / Draft)
    tui.comment("please rename greet", key=";c")  # also replaces the "Submitted" status line: a fresh barrier below
    tui.reject("Submitted GitHub review")
    mark = tui.forge_mark()
    tui.cmd("submit").wait_for("Submit review to GitHub").wait_for("> Comment")
    tui.keys("j", "j").wait_for("> Request changes").keys("Enter")  # a picked event submits without a confirm modal
    tui.wait_for("Submitted GitHub review #1001")
    posts = tui.posts(since=mark)
    assert len(posts) == 1, f"expected exactly one review POST for request-changes: {posts}"
    b = posts[0]["payload"]
    assert b["event"] == "REQUEST_CHANGES", b  # else: the picker's Request changes row sent another event
    assert "please rename greet" in b["body"], b
    assert "overall looks good" not in b["body"], f"the approved review's body was sent again: {b}"
    tui.quit()
