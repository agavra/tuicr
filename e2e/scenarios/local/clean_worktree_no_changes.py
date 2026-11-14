#!/usr/bin/env python3
# A committed, clean repository: `-w` (working tree) has nothing to review and exits 1 with "No changes to review",
# while
# the default startup still opens the selector listing the commit history. Dirtying the tree makes `-w` work.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.git(repo, "add", "-A")
    repos.git(repo, "commit", "-qm", "settle the work tree")
    r = s.cli_result("--no-update-check", "-w", cwd=repo)
    assert r.rc == 1, f"expected exit 1, got {r.rc}\n{r.out}"
    assert "No changes to review" in r.out, f"no empty-state message:\n{r.out}"
    tui = s.start(cwd=repo)
    tui.wait_for("settle the work tree").expect("fix: greeting").reject("Unstaged changes")
    tui.quit()
    with open(repo / "README.md", "a") as f:
        f.write("dirty\n")
    tui = s.start("-w", cwd=repo)
    tui.wait_for(r"README\.md").expect("dirty")
    tui.quit()
