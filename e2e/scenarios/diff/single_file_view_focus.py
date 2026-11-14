#!/usr/bin/env python3
# `:focus` shows one file at a time: `}` / `{` swap the file under review; `:focus` again returns to the continuous diff.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.wait_for("delete me")  # continuous: README.md and gone.txt both rendered
    tui.cmd("focus").wait_for("FOCUS")
    tui.wait_gone("delete me").expect("second line")
    tui.keys("}").wait_for(r"\(binary file\)").reject("second line")
    tui.keys("}").wait_for("delete me").reject("binary file")
    tui.keys("{").keys("{").wait_for("second line")
    tui.cmd("focus").wait_gone("FOCUS").wait_for("delete me").expect("second line")
    tui.quit()
