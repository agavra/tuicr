#!/usr/bin/env python3
# Plain `:clip` (no --stdout) copies the review markdown to the system clipboard, confirms it, and leaves the TUI
# running.
from e2e import Scenario, grep, scenario


@scenario
def main(s: Scenario) -> None:
    clipboard = s.fake_clipboard()
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.comment("clip me please")
    tui.cmd("clip")
    clip = clipboard.text()
    assert (
        grep("^I reviewed your code", clip)
        and grep("^## Local tuicr Comments", clip)
        and grep(r"^1\. `README\.md:4` - clip me please$", clip)
    ), f"clipboard lacks the structured export:\n{clip}"
    tui.wait_for("Review copied to clipboard")  # the status line confirms the copy
    # Still running (a dead pane fails every wait): the review is still shown and keys still move the cursor.
    tui.settle().check("clip me please", "the review is no longer shown after :clip")
    tui.cursor_on(r"═══ data\.bin")
    tui.quit()
