#!/usr/bin/env python3
# A repository with no commits and no changes has nothing to review: tuicr exits 1 with "No changes to review" (no empty
# TUI). Adding a file makes the same repository reviewable: the selector then offers the unstaged change.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.fresh(s.dir / "repo")
    repos.run(repo, "git", "init", "-q", "-b", "main", str(repo))
    r = s.cli_result("--no-update-check", cwd=repo)
    assert r.rc == 1, f"expected exit 1, got {r.rc}\n{r.out}"
    assert "No changes to review" in r.out, f"no empty-state message:\n{r.out}"
    repos.write(repo / "new.txt", "first file\n")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes").keys("Enter")
    tui.wait_for(r"new\.txt").expect("first file")
    tui.quit()
