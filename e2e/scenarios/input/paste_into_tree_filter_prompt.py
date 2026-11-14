#!/usr/bin/env python3
# Covers #900: a bracketed paste into the file-tree `i` filter prompt lands in the draft (`└ i README` on the tree border).
# Baseline's paste table only knew comment/command/search modes and silently dropped the paste.
from e2e import Scenario, because, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.type(";h").settle(0.2).type(";h").settle(0.2)
    tui.keys("i").wait_for("└ i ", 3)
    tui.paste("README")
    with because("pasted text did not reach the file-tree filter prompt"):
        tui.expect("└ i README", 3)
    tui.keys("Enter")
    tui.expect("Files · 0/1", 3)
    tui.quit()
