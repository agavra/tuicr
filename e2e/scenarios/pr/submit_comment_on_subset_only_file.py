#!/usr/bin/env python3
# #868: submit maps comments against the displayed (selected-range) diff. src/greet.rs exists
# only in the newest commit's diff, not in the cumulative one (variant subset_only_file); a comment on
# it must go out inline, not be sent to the "unmappable" resolver.
from e2e import Scenario, pr_comment, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open("subset_only_file")
    tui.keys(";k").keys("Enter")  # narrow to the newest commit (2222222)
    tui.wait_for(r"Commit Message \(2222222\)").wait_for(r"greet\.rs")
    tui.keys(";l").settle()
    pr_comment(tui, "inline on subset-only file")
    tui.cmd("submit comment")
    tui.wait_for("Submit review to GitHub|nmappable|Resolve")
    tui.expect("Submit review to GitHub").expect("Inline: 1")
    tui.keys("y")
    tui.wait_for("Submitted GitHub review #1001")
    p = tui.posts()[0]["payload"]
    got = [(c["path"], c["line"], c["body"]) for c in p["comments"]]
    assert got == [("src/greet.rs", 1, "inline on subset-only file")], got
    tui.quit()
