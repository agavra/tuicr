#!/usr/bin/env python3
# Covers #912: a reviewed mark made on the whole range must survive narrowing to one commit and widening again.
# Baseline re-registered the file with the subset's content hash, which invalidated the mark (Files x/N fell back to 0).
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.fresh(s.dir / "repo")
    repos.run(r, "git", "init", "-q", "-b", "main", str(r))
    repos.write(r / "a.txt", "one\n")
    repos.write(r / "b.txt", "x\n")
    repos.git(r, "add", "-A")
    repos.git(r, "commit", "-qm", "c1")
    repos.write(r / "a.txt", "one\ntwo\n")
    repos.write(r / "b.txt", "x2\n")
    repos.git(r, "commit", "-qam", "c2")
    repos.write(r / "a.txt", "one\ntwo\nthree\n")
    repos.git(r, "commit", "-qam", "c3")
    tui = s.start("-r", "HEAD~2..HEAD", cwd=r)
    tui.wait_for("Files · 0/2").keys("Escape")
    tui.cursor_on("three").keys("{").settle()
    tui.keys("r").wait_for("Files · 1/2")
    tui.keys(")").wait_for(r"Commit Message \(").settle()
    tui.expect("Files · 1/")  # reviewed mark lost after narrowing to one commit otherwise
    tui.keys(")").wait_gone(r"Commit Message \(").settle()
    tui.expect("Files · 1/2")  # reviewed mark lost after widening back otherwise
    tui.quit()
