#!/usr/bin/env python3
# jj backend: `-r @-` reviews the working copy's parent (adds, edits and a deletion with their own hunks); a
# Space-selected range in the selector reviews both commits cumulatively. Needs real jj.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    s.use_jj()
    repo = repos.jj_repo(s.dir / "repo")
    tui = s.start("-r", "@-", cwd=repo)
    tui.wait_for("Files · 0/4").expect(r"M a\.txt").expect(r"D b\.txt").expect(r"A c\.txt")
    tui.expect("c one").expect("a two").reject("a wip|d wip")  # nothing of the working-copy change
    tui.quit()
    rng = s.start(cwd=repo, name=f"{s.name}_range")
    rng.wait_for("wip").keys("Space", "Space", "Enter")  # rows: wip, second -> both
    rng.wait_for("Files · 0/4").expect(r"A c\.txt").expect(r"A d\.txt").expect(r"D b\.txt")
    rng.expect("a wip").expect("a two")
    rng.quit()
