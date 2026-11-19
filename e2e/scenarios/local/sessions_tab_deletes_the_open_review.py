#!/usr/bin/env python3
# Sessions tab: `dd` deletes the saved review this very TUI has open, and the review does not come back (#865).
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    # Leave only the unstaged change, so the next start loads this same
    # review instead of creating one for a different target.
    repos.git(repo, "reset", "-q")
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
    tui.keys("y")
    tui.wait_gone("1 comments")
    assert not s.session_has("delete me"), "dd left the session this TUI has open"
    # A later save starts a new review: the deleted comments must not return.
    tui.keys("Tab").wait_for("Unstaged changes").keys("Enter")
    tui.wait_for("unstaged line").cursor_on("unstaged line")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("new comment").keys("Enter")
    tui.wait_gone("COMMENT")  # the box closes only after the autosave lands

    assert s.session_has("new comment"), "the fresh comment was not saved"
    assert not s.session_has("delete me"), "the deleted review came back"
    tui.quit()
