#!/usr/bin/env python3
# Started outside any repository, tuicr exits 1 before the TUI with a "Not a repository" diagnostic
# (docs/CLI.md exit codes).
from e2e import Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    plain = repos.fresh(s.dir / "plain")
    r = s.cli_result("--no-update-check", cwd=plain)
    assert r.rc == 1, f"expected exit 1, got {r.rc}\n{r.out}"
    assert grep("Not a repository", r.out), f"no repository diagnostic:\n{r.out}"
    assert grep("git, jujutsu, or mercurial repository", r.out), f"no hint about supported VCSs:\n{r.out}"
    # the same for a revision range: still the repository error, not a revision error
    r = s.cli_result("--no-update-check", "-r", "HEAD~1..HEAD", cwd=plain)
    assert r.rc == 1 and grep("Not a repository", r.out), f"-r outside a repo: rc={r.rc}\n{r.out}"
