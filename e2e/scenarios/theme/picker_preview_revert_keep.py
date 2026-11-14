#!/usr/bin/env python3
# :theme opens the picker; j previews live (colours change), Esc reverts, Enter keeps; :theme <name> applies directly.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    # colour-aware snapshot of the whole pane, minus the popup rows (compare only the status line + header rows)
    base = tui.colors_hash(1, 45)
    tui.cmd("theme")
    tui.wait_for("Switch Theme").expect("gruvbox-dark").expect("nord-dark")
    p0 = tui.colors_hash(1, 45)
    tui.keys("j").settle().keys("j").settle()
    p2 = tui.colors_hash(1, 45)
    assert p0 != p2, "live preview did not change colours"
    tui.keys("Escape").wait_gone("Switch Theme")
    tui.settle()
    assert tui.colors_hash(1, 45) == base, "Esc did not revert the preview"
    # Enter keeps the previewed theme
    tui.cmd("theme").wait_for("Switch Theme").keys("j").settle().keys("Enter")
    tui.wait_gone("Switch Theme").settle()
    assert tui.colors_hash(1, 45) != base, "Enter did not keep the theme"
    # direct name, and unknown name
    tui.cmd("theme nord-dark").settle().expect("Files · 0/8")
    tui.cmd("theme no-such-theme-xyz")
    tui.wait_for("[Uu]nknown|not found|[Ii]nvalid|No theme|no such")
    tui.quit()
