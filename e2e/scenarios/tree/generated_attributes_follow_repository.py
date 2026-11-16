#!/usr/bin/env python3
# Covers #874: resuming an unrelated PR must discard the previous checkout's generated-file attributes.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.git(repo, "remote", "add", "origin", "https://github.com/octo/demo.git")
    foreign = s.start(
        "pr",
        "https://github.com/other/project/pull/1",
        cwd=repo,
        name=s.name + "_foreign",
        env={"XDG_DATA_HOME": str(s.dir / "data")},
    )
    foreign.wait_for("PR Description", 20).cursor_on("new line").comment("foreign PR session")
    foreign.quit()
    repos.git(repo, "remote", "set-url", "origin", "https://github.com/other/project.git")
    repos.write(repo / ".gitattributes", "README.md linguist-generated\n")
    tui = s.start("-w", cwd=repo, keep=True)
    tui.wait_for(r"README\.md.*\[generated\]").reject("unstaged line")
    tui.cmd("sessions").wait_for("1 comments")
    # Keep the saved PR listed, but make the checkout foreign before it is opened.
    repos.git(repo, "remote", "set-url", "origin", "https://github.com/octo/demo.git")
    tui.keys("Enter").wait_for("PR Description", 20)
    tui.wait_for("new line")
    tui.reject(r"README\.md.*\[generated\]")
    tui.quit()
