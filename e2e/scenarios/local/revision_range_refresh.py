#!/usr/bin/env python3
# Issue #1007: `:e` on a symbolic revision range re-resolves it, so commits made while tuicr is open appear in the
# same review. Reviewed marks stay on unchanged files and reset when a file changed.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    repos.write(r / "a.txt", "a1\n")
    repos.write(r / "b.txt", "b1\n")
    repos.git(r, "add", "-A")
    repos.git(r, "commit", "-qm", "base")
    repos.git(r, "branch", "base")

    repos.write(r / "a.txt", "a1\na2\n")
    repos.git(r, "commit", "-qam", "change a")
    repos.write(r / "b.txt", "b1\nb2\n")
    repos.git(r, "commit", "-qam", "change b")

    tui = s.start("-r", "base..HEAD", cwd=r)
    tui.wait_for("Files · 0/2").keys("Escape")
    tui.keys("r").wait_for("Files · 1/2")
    tui.keys("}").keys("r").wait_for("Files · 2/2")

    # A new commit lands while the review is open: a.txt changes and c.txt is added.
    repos.write(r / "a.txt", "a1\na2\na3\n")
    repos.write(r / "c.txt", "c1\n")
    repos.git(r, "add", "-A")
    repos.git(r, "commit", "-qm", "change a again and add c")

    tui.cmd("e").wait_for("new commits")
    # b.txt is unchanged and keeps its mark; a.txt changed and was reset; c.txt is new.
    tui.wait_for("Files · 1/3").expect(r"b\.txt").expect(r"c\.txt")
    tui.quit()
