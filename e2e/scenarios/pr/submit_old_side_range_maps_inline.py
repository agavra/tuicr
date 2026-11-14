#!/usr/bin/env python3
# Covers #902: an old-side range over a context line + a deleted line (README L1-L2) maps to an inline GitHub
# comment (start_line 1, line 2, side LEFT). Baseline calls it "range spans both diff sides" and sends it to the
# resolver.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open()
    tui.keys("}").cursor_on("old line")
    tui.keys("v").keys("k").settle().keys("c").wait_for("COMMENT", 3)
    tui.keys("range-old").keys("Enter").wait_for("L1-L2")
    tui.cmd("submit comment").wait_for("Submit review to GitHub|cannot be posted inline")
    tui.reject("cannot be posted inline")  # else: old-side range over context+deletion went to the resolver
    tui.wait_for("Submit review to GitHub").keys("y")
    tui.wait_for("Submitted GitHub review", 15)
    c = tui.posts()[0]["payload"]["comments"]
    print(c)
    assert len(c) == 1 and c[0]["side"] == "LEFT" and c[0]["start_line"] == 1 and c[0]["line"] == 2, c
    tui.quit()
