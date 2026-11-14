#!/usr/bin/env python3
# :diff toggles side-by-side and back.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cmd("diff")
    # SBS: old and new text of the same change on one row, split by a divider
    tui.wait_for("3  first line +│ +3  first line")
    tui.cmd("diff")
    tui.wait_gone("3  first line +│ +3  first line")
    tui.expect("4 ▌ second line")
    tui.quit()
