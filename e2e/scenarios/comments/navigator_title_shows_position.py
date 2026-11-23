#!/usr/bin/env python3
# Covers #954: the comment navigator title shows the selected comment's position before the total
# (`Comments · 2/3`), following the selection in the navigator and the diff cursor landing on a comment.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    # one file comment in each of the first three files, so the navigator lists three rows
    tui.comment("first note", key="C")
    tui.keys("}").settle().comment("second note", key="C")
    tui.keys("}").settle().comment("third note", key="C")
    tui.wait_for(r"Comments · [123]/3")

    # focus the navigator (Tab: diff -> commit selector -> comments) and walk it from the top
    tui.keys("Tab").settle().keys("Tab").settle()
    tui.keys("k", "k", "k").wait_for(r"Comments · 1/3")
    tui.keys("j").wait_for(r"Comments · 2/3")
    tui.keys("j").wait_for(r"Comments · 3/3")
    tui.keys("j").settle().check(r"Comments · 3/3", "the last row must stay selected at the end of the list")

    # Enter jumps to the third comment in the diff; from there the diff cursor drives the selection
    tui.keys("Enter").wait_for("j/k scroll").check(r"Comments · 3/3", "Enter must keep the jumped-to row selected")
    tui.keys("M").wait_for(r"Comments · 2/3")
    tui.keys("M").wait_for(r"Comments · 1/3")
    tui.quit()
