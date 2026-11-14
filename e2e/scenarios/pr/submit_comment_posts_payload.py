#!/usr/bin/env python3
# :submit comment -> confirm modal -> POST payload event=COMMENT with the inline comment; cancel posts nothing.
from e2e import pr_comment, scenario


@scenario
def main(s):
    tui = s.pr_open()
    pr_comment(tui, "inline for submit")
    # cancel first
    tui.cmd("submit comment").wait_for("Submit review to GitHub").expect("Event: Comment").expect("Inline: 1")
    tui.keys("n").wait_gone("Submit review to GitHub")
    assert tui.posts() == [], f"POST happened after cancel: {tui.posts()}"
    # then confirm
    tui.cmd("submit comment").wait_for("Submit review to GitHub").keys("y")
    tui.wait_for("Submitted GitHub review #1001")
    posts = tui.posts()
    print(posts)
    assert len(posts) == 1, "expected exactly one POST"
    p = posts[0]
    b = p["payload"]
    assert p["path"] == "repos/octo/demo/pulls/1/reviews", p["path"]
    assert b["event"] == "COMMENT", b
    assert b["commit_id"] == "2" * 40, b
    assert [(c["path"], c["line"], c["side"], c["body"]) for c in b["comments"]] \
        == [("src/greet.rs", 1, "RIGHT", "inline for submit")], b["comments"]
    tui.quit()
