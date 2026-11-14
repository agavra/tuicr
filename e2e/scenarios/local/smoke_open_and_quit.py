#!/usr/bin/env python3
# Open the git fixture, see the file tree + a diff, quit with :q.
from e2e import TUICR_BIN, Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start_cmd([TUICR_BIN], cwd=repo)  # no --no-update-check: the bare binary starts
    tui.wait_for("Staged changes|Unstaged changes")  # commit selector
    tui.keys("Enter")
    tui.expect(r"README\.md")  # file tree
    tui.expect("unstaged line")  # diff content
    tui.keys(":q", "Enter")  # `q` alone is only a hint
    tui.wait_exit()
