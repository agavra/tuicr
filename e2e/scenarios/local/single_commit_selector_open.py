#!/usr/bin/env python3
# Opening ONE historical commit straight from the startup selector (Enter on a commit row, no Space range) reviews
# exactly that commit: its message and its own files, nothing from later commits or from the working tree.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes")
    tui.keys("j", "j", "j")  # rows: unstaged, staged, fix, feat, base -> cursor on "feat"
    tui.keys("Enter")
    tui.wait_for(r"Commit Message \([0-9a-f]+\)")
    tui.expect("feat: touch every file kind").expect("Files · 0/8")
    tui.expect(r"▢ M README\.md").expect(r"▢ D gone\.txt")
    tui.keys("Escape")
    tui.expect("second line")  # README.md's change from this very commit
    tui.reject("unstaged line")  # working-tree edit
    tui.reject(r"staged\.txt")
    tui.reject("fix: greeting")  # a later commit
    tui.reject(r"src/main\.rs")  # only touched by the later commit
    tui.quit()
