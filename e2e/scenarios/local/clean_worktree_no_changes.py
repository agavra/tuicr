#!/usr/bin/env python3
# Covers #872 / #696: clean Git targets keep selectable empty rows in the selector and inline pane.
# Explicit -w still reports no changes; a new edit makes it usable.
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
    tui.wait_for("settle the work tree").expect("fix: greeting")
    tui.expect(r"Unstaged changes.*\(no changes\)").expect(r"Staged changes.*\(no changes\)")
    tui.keys("k", "k", "Enter").wait_for("No unstaged changes")
    tui.cmd("commits").wait_for("Unstaged changes")
    tui.keys("k", "Enter").wait_for("No staged changes")
    tui.cmd("commits").wait_for("settle the work tree")
    tui.keys("Enter").wait_for("Commit Message")
    tui.expect("settle the work tree")
    tui.quit()
    inline = s.start("-r", "HEAD~2..HEAD", "-w", cwd=repo)
    inline.wait_for("Files ·").expect(r"Unstaged changes.*\(no changes\)")
    inline.expect(r"Staged changes.*\(no changes\)")
    inline.keys(";k", "k", "k", "k", "k", "Enter").wait_for("No staged changes")
    inline.cmd("commits").wait_for("settle the work tree")
    inline.quit()
    with open(repo / "README.md", "a") as f:
        f.write("dirty\n")
    tui = s.start("-w", cwd=repo)
    tui.wait_for(r"README\.md").expect("dirty")
    tui.quit()
