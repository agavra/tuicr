#!/usr/bin/env python3
# Open PR #1 through the fake gh, see title + a diff line, quit.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    tui = s.start("pr", "1", cwd=repo)
    tui.wait_for("Add greeting and tweak README")
    tui.expect("hello from the PR")
    tui.forge_expect('"pr", "view", "1"')
    tui.keys(":q", "Enter")  # `q` alone is only a hint
    tui.wait_exit()
