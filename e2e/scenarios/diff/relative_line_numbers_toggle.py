#!/usr/bin/env python3
# `:set relativenumber` numbers rows by distance from the cursor (the cursor row keeps its own line number); `;n` and
# `:set norelativenumber` restore absolute numbers.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.expect("^│.*│ +3 +first line")  # absolute: line 3
    tui.cmd("set relativenumber")
    tui.wait_for("│ +1 +first line").expect("│ +3 +# demo").expect("▶ +4 ▌ second line")
    tui.cmd("set norelativenumber")
    tui.wait_for("│ +3 +first line").expect("│ +1 +# demo")
    tui.keys(";n").wait_for("│ +1 +first line")  # <leader>n toggles it on
    tui.keys(";n").wait_for("│ +3 +first line")
    tui.quit()
