#!/usr/bin/env python3
# :set wrap! wraps long lines; toggling back truncates them again.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(repo / "long.txt", "word " * 60 + "END\n")
    repos.git(repo, "add", "long.txt")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes").keys("j", "Enter")
    tui.wait_for(r"long\.txt")
    tui.cmd("set wrap")
    tui.wait_for("END")  # wrapped: the tail of the long line is visible
    tui.cmd("set wrap!")
    tui.wait_gone("END")  # toggled off: truncated
    tui.cmd("wrap")
    tui.wait_for("END")  # :wrap toggles back on
    tui.quit()
