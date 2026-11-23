#!/usr/bin/env python3
# `tuicr pr` on a branch no open PR uses fails before the TUI, pointing at `tuicr pr <number>`. The fake gh lists PRs on
# feature/greet-v2, feature/greet and fix/feature: each contains `feature`, none is it, so none may be picked.
from e2e.tui import read_forge_log

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    repos.git(repo, "checkout", "-q", "-b", "feature")
    r = s.cli_result("--no-update-check", "pr", cwd=repo, env={"E2E_VARIANT": "branch_scan"})
    assert r.rc == 1, f"expected exit 1, got {r.rc}: {r.out}"
    assert "No open pull request found for branch `feature`" in r.out, f"no branch diagnostic: {r.out}"
    assert "tuicr pr <number>" in r.out, f"no hint: {r.out}"
    calls = [c["argv"][:2] for c in read_forge_log(s.dir / "forge.log")]
    assert ["pr", "list"] in calls and ["pr", "view"] not in calls, f"expected a PR list and no PR view: {calls}"
