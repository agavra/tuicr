#!/usr/bin/env python3
# :e reloads the working-tree diff; comments + reviewed marks survive a restart.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes").keys("Enter")
    tui.wait_for("unstaged line")
    # modify the file on disk, :e picks it up
    with open(repo / "README.md", "a") as f:
        f.write("late edit from outside\n")
    tui.reject("late edit from outside", 1)
    tui.cmd("e")
    tui.wait_for("late edit from outside")
    # comment + reviewed, save, quit
    tui.cursor_on("late edit from outside")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("persist me").keys("Enter").wait_for("persist me")
    tui.keys("{").keys("r").wait_for("Files · 1/2")
    tui.save()
    tui.cmd("q!").wait_exit()
    # restart in the same repo: same selection, comment and reviewed mark come back
    tui = s.start(cwd=repo, keep=True)
    tui.wait_for("Unstaged changes").keys("Enter")
    tui.wait_for("Files · 1/2").expect(r"✓ README\.md")
    tui.cmd("summary").wait_for("SUMMARY").expect("persist me")
    tui.keys("Escape")
    tui.quit()
