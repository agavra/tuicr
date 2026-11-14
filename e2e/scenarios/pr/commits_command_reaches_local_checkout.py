#!/usr/bin/env python3
# Covers #916: from `tuicr pr 1` in a real checkout, `:commits` must reach the checkout's local targets
# (Unstaged changes, ...) and loading one must work. Baseline had swapped in the PR no-op VCS: `:commits` answered
# "No commits or ..." and the selector never opened.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.git(repo, "remote", "add", "origin", "https://github.com/octo/demo.git")
    tui = s.start("pr", "1", cwd=repo, env={"E2E_VARIANT": "threads_and_commits"})
    tui.wait_for("PR Description", 20)
    tui.cmd("commits")
    # :commits in PR mode did not open the local target selector
    tui.expect("Unstaged changes")
    tui.keys("Enter")
    # local Unstaged changes did not load after leaving PR mode
    tui.expect("unstaged line")
    tui.reject("PR Mode", 1)
    tui.quit()
