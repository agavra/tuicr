#!/usr/bin/env python3
# Sessions tab: a saved review is listed with its comment count and Enter resumes it (target and comments are back).
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes").keys("Enter")
    tui.wait_for("unstaged line").cursor_on("unstaged line")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("resume me").keys("Enter").wait_for("resume me")
    tui.quit()
    tui = s.start(cwd=repo, keep=True)
    tui.wait_for("Unstaged changes")
    tui.keys("Tab", "Tab").wait_for("Sessions")
    tui.wait_for("1 comments")
    tui.keys("Enter")
    tui.wait_for("unstaged line").wait_for("resume me")
    tui.quit()
