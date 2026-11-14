#!/usr/bin/env python3
# ? opens help, / searches it, q closes.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.keys("?").wait_for("HELP|Help|Keybindings")
    tui.expect("reviewed|comment")
    tui.keys("/").keys("visual").keys("Enter").settle()
    tui.keys("?")
    tui.wait_gone("HELP")
    tui.expect("Files · 0/8")
    tui.cmd("help").wait_for("HELP|Help")
    tui.keys("q").wait_gone("HELP")
    tui.quit()
