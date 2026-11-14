#!/usr/bin/env python3
# `-r <rev>` with a single revision (a ref, then a full SHA) reviews that commit against its parent, not the history
# up to it.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start("-r", "HEAD", cwd=repo)
    tui.wait_for(r"Commit Message \([0-9a-f]+\)").expect("fix: greeting").expect("Files · 0/2")
    tui.expect(r"src/main\.rs")
    tui.reject(r"README\.md").reject("feat: touch every file kind")  # an earlier commit
    tui.keys("Escape").expect("hello")
    tui.quit()
    # an explicit SHA, of the middle commit: its own seven files + message, not the later commit's
    sha = repos.git(repo, "rev-parse", "HEAD~1")
    tui = s.start("-r", sha, cwd=repo)
    tui.wait_for(r"Commit Message \([0-9a-f]+\)").expect("feat: touch every file kind").expect("Files · 0/8")
    tui.expect(r"▢ M README\.md").expect(r"▢ D gone\.txt")
    tui.reject("fix: greeting").reject(r"src/main\.rs")
    tui.quit()
