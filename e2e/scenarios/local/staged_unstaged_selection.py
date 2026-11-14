#!/usr/bin/env python3
# Selection rows: "Unstaged changes" / "Staged changes", picking one, and both with Space.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes").expect("Staged changes").expect("fix: greeting")
    # staged only
    tui.keys("j", "Enter")
    tui.expect(r"▢ A staged\.txt").expect("staged addition").reject(r"README\.md|untracked")
    # back to selector, unstaged only
    tui.keys("Escape").keys(":commits", "Enter").wait_for("Unstaged changes")
    tui.keys("Enter")
    tui.expect(r"▢ M README\.md").expect("unstaged line").expect(r"▢ A untracked\.txt").reject("staged addition")
    # both rows (Space on each) -> union
    tui.keys("Escape").keys(":commits", "Enter").wait_for("Unstaged changes")
    tui.keys("Space", "j", "Space", "Enter")
    tui.expect(r"▢ A staged\.txt").expect(r"▢ M README\.md")
    tui.quit()
