#!/usr/bin/env python3
# Sessions tab: `dd` on a saved review asks first (`n` keeps it), `y` deletes it and its session file.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    tui = s.start(cwd=repo)
    tui.wait_for("Unstaged changes").keys("Enter")
    tui.wait_for("unstaged line").cursor_on("unstaged line")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("delete me").keys("Enter").wait_for("delete me")
    tui.quit()
    tui = s.start(cwd=repo, keep=True)
    tui.wait_for("Unstaged changes")
    tui.keys("Tab", "Tab").wait_for("1 comments")
    tui.keys("d", "d").wait_for(r"Delete this saved review\?")
    tui.keys("n").wait_gone("Delete this saved review")
    tui.expect("1 comments")
    assert s.session_has("delete me"), "declining the prompt deleted the session"
    tui.keys("d", "d").wait_for(r"Delete this saved review\?")
    tui.keys("y")
    tui.wait_gone("1 comments")
    assert not s.session_has("delete me"), "dd left the session file"
    tui.quit()
