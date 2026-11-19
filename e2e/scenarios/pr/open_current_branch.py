#!/usr/bin/env python3
# `tuicr pr` with no target opens the PR whose head is the checked-out branch (fake gh lists PR #1 on feature/greet).
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    repos.git(repo, "checkout", "-q", "-b", "feature/greet")
    tui = s.start("pr", cwd=repo)
    tui.wait_for("Add greeting and tweak README")
    tui.expect("hello from the PR")
    tui.forge_expect('"pr", "list"')
    tui.forge_expect('"pr", "view", "1"')
    tui.keys(":q", "Enter")  # `q` alone is only a hint
    tui.wait_exit()
