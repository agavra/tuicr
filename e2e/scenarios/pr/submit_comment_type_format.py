#!/usr/bin/env python3
# Covers #945: `forge.comment_type_format` shapes the type prefix on submitted comments, e.g. Conventional Comments
# style `**nitpick (non-blocking):** ` instead of the default `[NITPICK (NON-BLOCKING)] `.
from e2e import Scenario, pr_comment, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repos.write(
        s.dir / "config/tuicr/config.toml",
        """comment_types = [{ id = "nit", label = "nitpick (non-blocking)" }]

[forge]
comment_type_format = "**{label}:** "
""",
    )
    tui = s.pr_open(keep=True)
    pr_comment(tui, "inline for submit")
    tui.cmd("submit comment").wait_for("Submit review to GitHub").keys("y")
    tui.wait_for("Submitted GitHub review #1001")
    posts = tui.posts()
    print(posts)
    assert len(posts) == 1, "expected exactly one POST"
    assert [c["body"] for c in posts[0]["payload"]["comments"]] == ["**nitpick (non-blocking):** inline for submit"], (
        posts[0]["payload"]["comments"]
    )
    tui.quit()
