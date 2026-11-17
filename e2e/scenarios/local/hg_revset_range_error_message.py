#!/usr/bin/env python3
# A malformed revset is diagnosed against the string the user wrote, not the `sort(..., rev)` wrapper
# resolve_revision_range passes to hg (`hg log -r sort(foo(, rev)` would report `sort(...)` instead of
# `foo(`). Needs real Mercurial: SKIP without it, FAIL with E2E_REQUIRE_HG=1.
from e2e import Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    s.use_hg()
    repo = repos.fresh(s.dir / "repo")
    repos.run(repo, "hg", "init", "-q", ".")

    r = s.cli_result("--no-update-check", "-r", "foo(", cwd=repo)
    assert r.rc == 1, f"expected exit 1, got {r.rc}\n{r.out}"
    assert grep(r"hg log -r foo\(", r.out), f"error does not quote the user's revset:\n{r.out}"
    assert not grep(r"sort\(", r.out), f"error quotes the sort wrapper:\n{r.out}"
