#!/usr/bin/env python3
# Covers #900: with the file-tree `i` prompt open, a mouse press in the diff must not steal input from the prompt.
# Baseline moved panel focus to the diff while the prompt still captured keys, so Esc hid the sidebar (file list) instead of
# cancelling the prompt.
from e2e import Scenario, because, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.type(";h").settle(0.2).type(";h").settle(0.2)
    tui.keys("i").wait_for("└ i ", 3)
    tui.type("ab").wait_for("└ i ab", 3)
    # SGR press+release in the diff pane (col 60,row 20)
    tui.click(60, 20).settle()
    tui.keys("Escape").settle()
    with because(
        "Esc after a diff click hid the file list instead of cancelling the prompt"
    ):
        tui.expect("Files · 0/8")
    tui.reject("└ i ab", 1)
    tui.quit()
