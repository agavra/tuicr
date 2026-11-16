#!/usr/bin/env python3
# The Pull Requests tab capped titles at 60 cells, so on a wide terminal each row ended mid-screen and a longer
# title was cut with room to spare. At 200 columns the whole title must show, followed by its author.
import re

from e2e import Scenario, repos, scenario

TITLE = "Resolve revision ranges oldest-first and quote the user's revset in every error message"


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    tui = s.start(cwd=repo, cols=200, env={"E2E_VARIANT": "long_title"})
    tui.wait_for("Pull Requests").settle().keys("Tab").wait_for("#1 ")
    tui.expect(re.escape(TITLE) + " +alice", 2)  # else: the title was cut although the row had room
    tui.quit()
