#!/usr/bin/env python3
# Covers #883 / #697: unified/SBS gutters identify each staged/unstaged edit and refresh after external staging.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.empty_repo(s.dir / "repo")
    path = repo / "markers.txt"
    original = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n"
    repos.write(path, original)
    repos.git(repo, "add", ".")
    repos.git(repo, "commit", "-qm", "base")
    staged = original.replace("two", "STAGED_ONLY")
    repos.write(path, staged)
    repos.git(repo, "add", "markers.txt")
    repos.write(path, staged.replace("six", "UNSTAGED_ONLY"))
    repos.write(s.dir / "config/tuicr/config.toml", "diff_watch_interval_ms = 200\n")
    tui = s.start("-w", cwd=repo, keep=True)
    tui.wait_for("UNSTAGED_ONLY")
    tui.expect("2●.*STAGED_ONLY").expect("6○.*UNSTAGED_ONLY")
    tui.cmd("diff").wait_for("│.*2●.*STAGED_ONLY")
    tui.expect("│.*6○.*UNSTAGED_ONLY")
    repos.git(repo, "add", "markers.txt")
    tui.wait_for("│.*6●.*UNSTAGED_ONLY", 15)
    tui.cmd("diff").wait_for("6●.*UNSTAGED_ONLY")
    tui.reject("○.*UNSTAGED_ONLY")
    tui.quit()
