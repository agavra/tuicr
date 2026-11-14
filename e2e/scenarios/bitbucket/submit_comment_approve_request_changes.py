#!/usr/bin/env python3
# Bitbucket Cloud happy paths: a submit is a series of plain comments (summary, then each inline one with its `inline`
# anchor,
# never `pending`); approve is `bkt pr approve`, request-changes the `/request-changes` endpoint.
from e2e import Scenario, pr_comment, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.forge_open("https://bitbucket.org/octo/demo.git")
    pr_comment(tui, "inline bb note")
    tui.keys(";c").wait_for("COMMENT").keys("bb overall").keys("Enter").wait_for("bb overall")
    tui.cmd("submit comment").wait_for("Submit review to Bitbucket").expect("Event: Comment").keys("y")
    tui.wait_for("Submitted Bitbucket review")
    calls = tui.forge_posts("bkt")
    comments = "/2.0/repositories/octo/demo/pullrequests/1/comments"
    assert [c["argv"][1] for c in calls] == [comments] * 2, [c["argv"] for c in calls]
    assert calls[0]["payload"] == {"content": {"raw": "bb overall"}}, calls[0]
    inline = {
        "content": {"raw": "inline bb note"},
        "inline": {"path": "src/greet.rs", "to": 1},
    }
    assert calls[1]["payload"] == inline, calls[1]
    tui.cmd("submit approve").wait_for("Submit review to Bitbucket").expect("Event: Approve").keys("y")
    tui.wait_forge_log('"pr", "approve", "1"')
    tui.cmd("submit request-changes").wait_for("Submit review to Bitbucket").expect("Event: Request changes").keys("y")
    tui.wait_forge_log("pullrequests/1/request-changes")
    tui.quit()
