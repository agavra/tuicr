#!/usr/bin/env python3
# jj backend: the selector lists jj commits (working copy first); Enter reviews the working-copy change with its files
# and hunks; a comment saves under the jj session. Needs real jj (SKIP without it, FAIL with E2E_REQUIRE_JJ=1).
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    s.use_jj()
    repo = repos.jj_repo(s.dir / "repo")
    tui = s.start(cwd=repo)
    tui.wait_for("wip").expect("second").expect("base").expect("jj:")
    tui.keys("Enter")
    tui.wait_for("Files · 0/3").expect(r"M a\.txt").expect(r"A d\.txt").expect("a wip").expect("d wip")
    tui.reject("c one")  # c.txt belongs to "second", not to the working copy
    tui.keys("Escape").cursor_on("a wip")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("jj note").keys("Enter").wait_for("jj note")
    tui.save()
    assert s.session_has("jj note"), "jj session not saved"
    tui.quit()
