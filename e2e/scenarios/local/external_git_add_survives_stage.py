#!/usr/bin/env python3
# a `git add` done outside tuicr while it runs must stay staged after :stage.
from e2e import Scenario, repos, scenario, wait_cmd


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")

    def cached() -> list[str]:
        return repos.git(repo, "diff", "--cached", "--name-only").splitlines()

    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes")
    tui.keys("Enter")
    tui.wait_for(r"▢ M README\.md")
    # external staging after tuicr already read the index
    repos.write(repo / "ext.txt", "external\n")
    repos.git(repo, "add", "ext.txt")
    assert "ext.txt" in cached()
    tui.keys("Escape").keys("j")  # focus file tree, README row
    tui.keys("r").wait_for("Files · 1/")
    tui.cmd("stage")

    def readme_staged() -> bool:  # barrier: :stage wrote README.md into the index
        return "README.md" in cached()

    wait_cmd(10, readme_staged)
    tui.settle()
    names = cached()
    print(f"cached: {names}")
    assert "README.md" in names, f"README.md was not staged\n{tui.screen()}"
    assert "ext.txt" in names, "external git add (ext.txt) was lost by :stage"
    assert "staged.txt" in names, "staged.txt was lost by :stage"
    tui.quit()
