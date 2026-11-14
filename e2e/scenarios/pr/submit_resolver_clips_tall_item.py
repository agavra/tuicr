#!/usr/bin/env python3
# Covers #911: a resolver item taller than the list (500-char path, 16-row tty) must be clipped, not dropped.
# Baseline's wrapped Paragraph overran the popup and pushed the footer line (`Esc: cancel`) out; the fix clips the item.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    s.rows = 16
    tui = s.pr_open("long_file_path")
    tui.keys("}").settle().keys("}").settle()
    tui.keys("C").wait_for("COMMENT").type("long-note").keys("Enter").settle()
    tui.cmd("submit comment").wait_for("cannot be posted inline").settle()
    print("\n".join(line[33:150] for line in tui.screen().split("\n")))
    tui.expect(r"> \[x\] Move to summary", 2)  # else: tall resolver item was dropped instead of clipped
    tui.expect("Esc: cancel", 2)  # else: resolver footer missing
    tui.keys("Escape").quit()
