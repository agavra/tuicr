#!/usr/bin/env python3
# `:clip` on a review with no comments: tuicr says so instead of exporting an empty document; the
# clipboard stays untouched.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    clipboard = s.fake_clipboard()
    tui = s.diff()
    tui.cmd("clip")
    tui.wait_for("No comments to export")
    assert tui.alive(), "tuicr exited after :clip"
    assert not clipboard.copied, f"clipboard written for an empty review:\n{clipboard.path.read_text()}"
    tui.quit()
