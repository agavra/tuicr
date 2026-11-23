#!/usr/bin/env python3
# Emptying a watched staged review preserves wrap_lines across the empty transition.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(s.dir / "config/tuicr/config.toml", "diff_watch_interval_ms = 200\n")
    repos.write(repo / "long.txt", "word " * 60 + "TAIL_BEFORE\n")
    repos.git(repo, "add", "long.txt")

    tui = s.start(cwd=repo, keep=True)
    tui.wait_for("Unstaged changes").keys("j", "Enter").wait_for(r"long\.txt")
    tui.cmd("set wrap").wait_for("TAIL_BEFORE")

    repos.git(repo, "commit", "-qm", "commit staged")
    tui.wait_for("No staged changes", 15)

    repos.write(repo / "long.txt", "word " * 60 + "TAIL_AFTER\n")
    repos.git(repo, "add", "long.txt")
    tui.wait_for(r"long\.txt", 15).wait_for("TAIL_AFTER", 15)
    tui.quit()
