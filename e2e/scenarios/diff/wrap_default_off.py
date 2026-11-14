#!/usr/bin/env python3
# docs/CONFIG.md documents `wrap` default = false: long lines must be truncated on a fresh start.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(repo / "long.txt", "word " * 60 + "END\n")
    repos.git(repo, "add", "long.txt")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes").keys("j", "Enter")
    tui.wait_for(r"long\.txt").wait_for("word word")
    tui.reject("END", 1)
    tui.keys(":q!", "Enter").wait_exit()
