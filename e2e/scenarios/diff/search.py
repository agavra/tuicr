#!/usr/bin/env python3
# Diff search: / regex, match counter, n / N, Esc clears highlight, invalid regex error.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.keys("/").keys("println|second").keys("Enter")
    tui.wait_for(r"\[1/[0-9]+\]").expect(r"\[1/3\]")
    tui.keys("n").wait_for(r"\[2/3\]")
    tui.keys("n", "n").wait_for(r"\[1/3\]")  # wraps
    tui.keys("N").wait_for(r"\[3/3\]")
    tui.keys("Escape").wait_gone(r"\[[0-9]/3\]")
    tui.keys("n").wait_for(r"\[[0-9]/3\]")  # pattern kept
    # an invalid regex reports an error and clears the match highlight/counter
    tui.keys("/").keys("(unclosed").keys("Enter")
    tui.wait_for("Invalid regex")
    tui.wait_gone(r"\[[0-9]/3\]")
    tui.quit()
