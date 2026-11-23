#!/usr/bin/env python3
# Diff search: / regex, match counter, n / N move the cursor between matches, Esc clears highlight, invalid regex
# error.
from e2e import Scenario, grep, scenario

MATCH = "println|second"


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()

    def on_match() -> None:
        assert grep(MATCH, tui.cursor_row()), f"the cursor is not on a match: {tui.cursor_row()!r}"

    tui.keys("/").type(MATCH).keys("Enter")
    tui.wait_for(r"\[1/[0-9]+\]").expect(r"\[1/3\]")
    on_match()
    first = tui.cursor_row()
    tui.keys("n").wait_for(r"\[2/3\]")
    on_match()
    second = tui.cursor_row()
    assert second != first, f"n did not move the cursor off the first match: {first!r}"
    tui.keys("n", "n").wait_for(r"\[1/3\]")  # wraps
    assert tui.cursor_row() == first, f"wrapping did not return to the first match: {tui.cursor_row()!r}"
    tui.keys("N").wait_for(r"\[3/3\]")
    on_match()
    # the highlight is on the match rows away from the cursor; Esc takes it off them
    lines = tui.screen().split("\n")
    # diff rows only (the `▌` gutter): the status line holding the counter changes on Esc anyway
    rows = [i + 1 for i, ln in enumerate(lines) if grep(MATCH, ln) and "▌" in ln and "▶" not in ln]
    assert rows, f"no match row away from the cursor\n{tui.screen()}"
    lit = tui.colors_hash(*rows)
    tui.keys("Escape").wait_gone(r"\[[0-9]/3\]")
    assert tui.colors_hash(*rows) != lit, "Esc left the match rows' highlight on"
    tui.keys("n").wait_for(r"\[[0-9]/3\]")  # pattern kept
    # an invalid regex reports an error and clears the match highlight/counter
    tui.keys("/").type("(unclosed").keys("Enter")
    tui.wait_for("Invalid regex")
    tui.wait_gone(r"\[[0-9]/3\]")
    tui.quit()
