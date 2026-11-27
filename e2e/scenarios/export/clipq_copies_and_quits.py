#!/usr/bin/env python3
# `:clipq` copies the review like `:clip` and quits like `:q!`, without saving (#997). With nothing to copy it says so
# and stays open.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    clipboard = s.fake_clipboard()
    tui = s.diff()
    tui.cmd("clipq")
    tui.wait_for("No comments to export")
    assert tui.alive(), ":clipq quit with nothing to copy"
    tui.cursor_on("4 ▌ second line")
    tui.comment("copy me and leave")  # autosaved
    tui.keys("r").wait_for("Files · 1/8")  # unsaved, so `:q` would refuse
    tui.cmd("clipq").wait_exit()
    assert "`README.md:4` - copy me and leave" in clipboard.text(), f"not copied:\n{clipboard.text()}"
    assert not s.session_reviewed("README.md"), ":clipq saved the session"
