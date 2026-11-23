#!/usr/bin/env python3
# Covers #872 / #696: watcher clears an emptied staged review and restores later changes.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(s.dir / "config/tuicr/config.toml", "diff_watch_interval_ms = 200\n")
    tui = s.start(cwd=repo, keep=True)
    tui.wait_for("Unstaged changes").keys("j", "Enter").wait_for("staged addition")
    repos.git(repo, "commit", "-qm", "commit staged target")
    tui.wait_for("No staged changes", 15).wait_gone("staged addition")
    tui.cmd("commits").wait_for(r"Staged changes.*\(no changes\)")
    tui.keys("j", "Enter").wait_for("No staged changes")
    repos.write(repo / "staged.txt", "replacement staged line\n")
    repos.git(repo, "add", "staged.txt")
    tui.wait_for("replacement staged line", 15)
    tui.quit()
