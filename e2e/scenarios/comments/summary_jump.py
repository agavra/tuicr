#!/usr/bin/env python3
# :summary lists pending local comments; Enter jumps the diff cursor to the selected one; Esc returns without moving it.
from e2e import Scenario, scenario

TOP = r"▶ @@ -1,3 \+1,4 @@"  # the cursor on README.md's hunk header, the first diff row


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.comment("summary one")
    tui.keys("G").comment("summary two", key="C")  # a file comment on the last file, far from the first
    tui.keys("g").wait_for(TOP).wait_for(r"Comments · 1/2")  # neither comment selected yet: the first
    tui.cmd("summary")
    tui.wait_for("SUMMARY").expect("summary one").expect("summary two")
    tui.keys("j", "Enter")
    tui.wait_gone("SUMMARY")
    # the cursor left the top and landed on the second comment: the navigator follows it to 2/2
    tui.wait_for(r"Comments · 2/2").wait_gone(TOP).expect("summary two")
    tui.keys("g").wait_for(TOP)  # the navigator keeps its selection (2/2) off a comment
    top = tui.cursor_row()
    tui.cmd("summary").wait_for("SUMMARY")
    tui.keys("Escape").wait_gone("SUMMARY")
    # Esc jumps nowhere, not even to the first (preselected) comment
    tui.settle()
    assert tui.cursor_row() == top, f"Esc moved the diff cursor: {tui.cursor_row()!r}, want {top!r}"
    tui.quit()
