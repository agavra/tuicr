#!/usr/bin/env python3
# `-A` (--all-files) puts every tracked file in the review, including ones with no change.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start("-A", cwd=repo)
    # 8 tracked files; data.bin is binary and dropped from an all-files review, untracked.txt is not tracked
    tui.wait_for("Files · 0/7")
    tui.expect(r"▢ README\.md").expect(r"▢ +main\.rs").expect(r"▢ staged\.txt").expect(r"\+0 -0")
    tui.reject(r"data\.bin|untracked\.txt")
    tui.reject(r"▢ [MADR] ")  # no status letters: nothing is a change
    tui.quit()
