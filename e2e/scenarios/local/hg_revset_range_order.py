#!/usr/bin/env python3
# hg revision ranges resolve oldest-first however the range is written: `-r 0:2` and `-r 2:0` review the
# same cumulative diff (an ascending range used to resolve newest-first, turning the added lines into
# deletions). Needs real Mercurial: SKIP without it, FAIL with E2E_REQUIRE_HG=1.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    s.use_hg()
    repo = repos.fresh(s.dir / "repo")
    repos.run(repo, "hg", "init", "-q", ".")
    lines = ["alpha", "beta", "gamma"]
    for i, summary in enumerate(["c1", "c2", "c3"], start=1):
        repos.write(repo / "f.txt", "".join(f"{line}\n" for line in lines[:i]))
        if i == 1:
            repos.run(repo, "hg", "add", "-q", "f.txt")
        repos.run(repo, "hg", "commit", "-q", "-u", "e2e", "-m", summary)

    # Both range directions cover c1..c3, so f.txt appears whole (added from null) and none of its lines is
    # removed. The inverted order would diff c2 against c1 instead: `@@ -1,2 +1,1 @@` with beta removed.
    for i, revset in enumerate(["0:2", "2:0"]):
        tui = s.start("-r", revset, cwd=repo, name=f"{s.name}_{i}")
        tui.wait_for("Files · 0/1").wait_for(r"@@ -0,0 \+1,3 @@")
        tui.expect("alpha").expect("beta").expect("gamma")
        tui.quit()
