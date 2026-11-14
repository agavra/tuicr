#!/usr/bin/env python3
# Pull-request export before any submit: a draft written in the TUI is exported with the PR's URL and head SHA (so an
# agent
# can fetch the PR; the fixture's head is 222...2, shown short), the numbered comment anchored at its file:line, and
# no POST is made to the forge.
from e2e import Scenario, pr_comment, scenario


@scenario
def main(s: Scenario) -> None:
    clipboard = s.fake_clipboard()
    tui = s.pr_open()
    pr_comment(tui, "export this draft")
    tui.cmd("clip")
    clipboard.text()  # waits for the copy
    clip = clipboard.path.read_bytes().decode()  # bytes: read_text would normalise CRLF
    lines = clip.splitlines()
    assert "URL: https://github.com/octo/demo/pull/1" in lines, f"export lacks the PR URL:\n{clip}"
    assert "Head: 22222222" in lines, f"export lacks the PR head sha:\n{clip}"
    assert "## Local tuicr Comments" in lines, f"export lacks the local comments heading:\n{clip}"
    assert "1. `src/greet.rs:1` - export this draft" in lines, f"export lacks the numbered, anchored draft:\n{clip}"
    # the unresolved remote thread is listed separately, after the local draft
    assert "## Existing GitHub Comments" in lines, f"export lacks the remote-thread section:\n{clip}"
    assert tui.posts() == [], f"a POST happened during export: {tui.posts()}"
    tui.quit()
