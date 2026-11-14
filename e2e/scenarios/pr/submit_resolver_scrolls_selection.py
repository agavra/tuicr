#!/usr/bin/env python3
# Covers #911: the submit resolver lists more unmappable comments than fit; the selected row and the footer
# ("Esc: cancel") must stay visible while moving the cursor to the last row. Baseline rendered a plain Paragraph that
# clipped
# the tail (selection vanished, footer gone). 9 file-level comments on binary files (+1 on the commit message), 26-row
# tty.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    s.rows = 26
    tui = s.pr_open("many_binary_files")
    for i in range(1, 10):
        tui.keys("}").settle(0.2).keys("C").wait_for("COMMENT").type(f"note-{i}").keys("Enter").settle(0.2)
    tui.cmd("submit comment").wait_for("cannot be posted inline").settle()
    tui.keys(*["j"] * 9).settle()
    print("\n".join(line[33:150] for line in tui.screen().split("\n")[5:24]))
    tui.expect(r"> \[x\] Move to summary  Commit Message", 2)  # else: selected last resolver row is not visible
    tui.expect("Esc: cancel", 2)  # else: resolver footer scrolled away
    tui.keys("Escape").quit()
