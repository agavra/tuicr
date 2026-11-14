#!/usr/bin/env python3
# `-p PATH` reviews only that path of the working tree (staged and untracked files elsewhere are left out).
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start("-p", "README.md", cwd=repo)
    tui.wait_for(r"README\.md \[M\]").expect("unstaged line")  # a single file: no tree pane
    tui.reject(r"staged\.txt|untracked\.txt")
    tui.quit()
