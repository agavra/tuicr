#!/usr/bin/env python3
# `-A` (--all-files) puts every tracked file in the review, including ones with no change.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start("-A", cwd=repo)
    tui.wait_for("Files · 0/7")  # 7 tracked files, no status letters: nothing is a change
    tui.expect(r"▢ README\.md").expect(r"▢ +main\.rs").expect(r"▢ staged\.txt").expect(r"\+0 -0")
    tui.quit()
