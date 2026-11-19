#!/usr/bin/env python3
# `tuicr pr` on a branch no open PR uses fails before the TUI, pointing at `tuicr pr <number>`.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    repos.git(repo, "checkout", "-q", "-b", "unrelated-branch")
    r = s.cli_result("--no-update-check", "pr", cwd=repo)
    assert r.rc == 1, f"expected exit 1, got {r.rc}: {r.out}"
    assert "No open pull request found for branch `unrelated-branch`" in r.out, f"no branch diagnostic: {r.out}"
    assert "tuicr pr <number>" in r.out, f"no hint: {r.out}"
