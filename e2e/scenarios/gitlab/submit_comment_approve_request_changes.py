#!/usr/bin/env python3
# GitLab happy paths: `:submit comment` posts the summary as an MR note and the inline comment as a discussion with a
# position;
# `:submit approve` additionally approves; `:submit request-changes` runs the request-changes mutation. Nothing is a
# draft note.
from e2e import Scenario, pr_comment, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.forge_open("https://gitlab.com/octo/demo.git")
    pr_comment(tui, "inline gl note")
    tui.keys(";c").wait_for("COMMENT").keys("gl overall").keys("Enter").wait_for("gl overall")
    tui.cmd("submit comment").wait_for("Submit review to GitLab").expect("Event: Comment").keys("y")
    tui.wait_for("Submitted GitLab review")
    calls = tui.forge_posts("glab")
    eps = [c["argv"][1] for c in calls]
    assert eps == [
        "projects/octo%2Fdemo/merge_requests/1/notes",
        "projects/octo%2Fdemo/merge_requests/1/discussions",
    ], eps
    assert calls[0]["payload"] == {"body": "gl overall"}, calls[0]
    d = calls[1]["payload"]
    pos = d["position"]
    assert d["body"] == "inline gl note" and pos["new_path"] == "src/greet.rs" and pos["new_line"] == 1, d
    assert pos["position_type"] == "text" and len(pos["head_sha"]) == 40, pos
    # the comments are now locked (submitted): approving and requesting changes carry only the vote
    tui.cmd("submit approve").wait_for("Submit review to GitLab").expect("Event: Approve").keys("y")
    tui.wait_forge_log("merge_requests/1/approve")
    tui.cmd("submit request-changes").wait_for("Submit review to GitLab").expect("Event: Request changes").keys("y")
    tui.wait_forge_log("mergeRequestRequestChanges")
    tui.forge_reject('draft_notes", "--method", "POST"')
    tui.quit()
