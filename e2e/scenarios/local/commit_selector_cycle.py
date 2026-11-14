#!/usr/bin/env python3
# Commit selector: Space range, Enter confirm, ( ) cycle individual commits in the inline selector.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes")
    tui.keys(
        "j", "j", "Space", "Space", "Enter"
    )  # rows: unstaged, staged, fix, feat, base; Space advances -> fix + feat
    tui.wait_for("Files · 0/8").expect(r"commits/[0-9a-f]+\.\.[0-9a-f]+").expect(r"data\.bin")
    tui.expect("fix: greeting").expect("feat: touch every file kind")
    # ) / ( cycle: first commit alone, ..., then back to the whole range. A lone commit shows its message.
    tui.keys(")").wait_for(r"Commit Message \([0-9a-f]+\)").expect("Files · 0/8")  # feat alone
    tui.keys(")").wait_gone(r"Commit Message \(").expect(r"▢ M README\.md")  # whole range again
    tui.keys("(").wait_for(r"Commit Message \([0-9a-f]+\)").expect("Files · 0/2")  # fix alone
    tui.expect("fix: greeting")
    tui.keys("(").wait_gone(r"Commit Message \(").expect("Files · 0/8")
    # Enter narrows to the commit under the cursor (cursor is on the first row: fix)
    tui.keys(";k").keys("Enter").wait_for(r"Commit Message \(").expect("Files · 0/2")
    # Space widens the selection again
    tui.keys("j", "Space").wait_gone(r"Commit Message \(").expect("Files · 0/8")
    tui.quit()
