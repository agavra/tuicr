#!/usr/bin/env python3
# Adding a comment autosaves it, so a later unsaved change (a reviewed mark) is what `:q` guards while comments exist:
# `:q` refuses, `:w` then lets it through; with no comment at all, reviewed marks alone do not block `:q`.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("unsaved note").keys("Enter").wait_for("unsaved note")
    tui.cmd("q").wait_exit()  # the comment itself was autosaved: nothing left to lose
    dirty = f"{s.name}_dirty"
    tui = s.diff(name=dirty)
    tui.cursor_on("4 ▌ second line")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("saved note").keys("Enter").wait_for("saved note")
    tui.keys("r").wait_for("Files · 1/8")  # unsaved change while a comment exists
    tui.cmd("q")
    tui.wait_for("No write since last change")
    assert tui.alive(), ":q quit with an unsaved comment"
    tui.save()
    tui.cmd("q")
    tui.wait_exit()
    # review-only state (a reviewed mark, no comment) does not block :q
    marks = s.open(name=f"{dirty}_marks")
    marks.wait_for("Files · 0/8").keys("Escape")
    marks.keys("r").wait_for("Files · 1/8")
    marks.cmd("q").wait_exit()
