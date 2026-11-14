#!/usr/bin/env python3
# Gitea happy paths: `:submit comment` / `approve` / `request-changes` each POST one review with Gitea's own event names
# (COMMENT, APPROVED, REQUEST_CHANGES), the inline comment as a single-line `new_position`; request-changes carries a
# summary.
from e2e import Scenario, pr_comment, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.forge_open("https://gitea.example.com/octo/demo.git")
    pr_comment(tui, "inline gt note")
    tui.keys(";c").wait_for("ctrl-s save").keys("gt overall").keys("Enter").wait_for("Review comment added")
    tui.cmd("submit comment").wait_for("Submit review to Gitea").expect("Event: Comment").keys("y")
    tui.wait_for("Submitted Gitea review")
    tui.cmd("submit approve").wait_for("Submit review to Gitea").expect("Event: Approve").keys("y")
    tui.wait_forge_log('"POST".*pulls/1/reviews"', 15, count=2)
    tui.keys(";c").wait_for("ctrl-s save").keys("gt please fix").keys("Enter").wait_for("Review comment added")
    tui.cmd("submit request-changes").wait_for("Submit review to Gitea").expect("Event: Request changes").keys("y")
    tui.wait_forge_log('"POST".*pulls/1/reviews"', 15, count=3)
    calls = tui.forge_posts("tea")
    pl = [c["payload"] for c in calls]
    assert [p["event"] for p in pl] == ["COMMENT", "APPROVED", "REQUEST_CHANGES"], [p.get("event") for p in pl]
    assert all(c["argv"][-1] == "/repos/octo/demo/pulls/1/reviews" for c in calls), [c["argv"] for c in calls]
    assert pl[0]["body"] == "gt overall", pl[0]
    assert pl[0]["comments"] == [{"path": "src/greet.rs", "body": "inline gt note", "new_position": 1}], pl[0]
    assert pl[1]["comments"] == [] and pl[2]["body"] == "gt please fix", pl
    tui.quit()
