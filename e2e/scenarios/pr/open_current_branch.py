#!/usr/bin/env python3
# `tuicr pr` with no target opens the PR whose head is the checked-out branch. The fake gh lists #3 on feature/greet-v2
# first, then #1 on feature/greet and #2 on fix/feature: only the exact head-branch match (#1) may be opened.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    repos.git(repo, "checkout", "-q", "-b", "feature/greet")
    tui = s.start("pr", cwd=repo, env={"E2E_VARIANT": "branch_scan"})
    tui.wait_for("Add greeting and tweak README")
    tui.expect("hello from the PR")
    tui.forge_expect('"pr", "list"')
    tui.forge_expect('"pr", "view", "1"')
    views = [c["argv"][2] for c in tui.forge_calls(lambda c: c["argv"][:2] == ["pr", "view"])]
    assert set(views) == {"1"}, f"`tuicr pr` on feature/greet viewed PRs {views}, not only #1"
    tui.keys(":q", "Enter")  # `q` alone is only a hint
    tui.wait_exit()
