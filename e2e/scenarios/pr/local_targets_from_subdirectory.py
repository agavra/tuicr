#!/usr/bin/env python3
# Covers #916: `tuicr pr 1` started from a subdirectory must still scope the Sessions tab / local targets by the
# checkout root. Baseline stored the raw cwd (src/), so a session saved at the checkout root was not listed.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.git(repo, "remote", "add", "origin", "https://github.com/octo/demo.git")
    # 1. create a local session at the root (comment on the unstaged diff), save, quit
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes").keys("Enter").wait_for("unstaged line")
    tui.cursor_on("unstaged line").comment("root-session-comment")
    tui.save().cmd("q!").wait_exit()
    # 2. PR from src/, then the Sessions tab
    tui = s.start("pr", "1", cwd=repo / "src", keep=True, env={"E2E_VARIANT": "threads_and_commits"})
    tui.wait_for("PR Description", 20)
    tui.cmd("commits").wait_for("Unstaged changes")
    tui.keys("Tab").keys("Tab")
    # barrier: a row only the Sessions tab draws (`<kind> <target>  <n> comments · ...`); the tab names are always
    # on screen, so they prove nothing. The checkout root's local session, with its one comment, must be listed.
    tui.wait_for(r"local .*  1 comments · ")  # else: the root session is not listed from a subdirectory launch
    tui.reject("No saved reviews for this checkout")
    tui.keys("Escape").quit()
