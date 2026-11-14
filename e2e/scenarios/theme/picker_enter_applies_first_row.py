#!/usr/bin/env python3
# Covers #901: `:theme` then Enter without moving must apply the selected (first) row. Baseline only announced it
# ("Theme: dark") and kept the running theme. Start on --theme light; first picker row is `dark`.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = (
        s.open("--theme", "light", "-r", "HEAD~2..HEAD")
        .wait_for("Files · 0/8")
        .keys("Escape")
    )
    base = tui.colors_hash(1, (3, 30))
    tui.cmd("theme").wait_for("Switch Theme")
    tui.keys("Enter").wait_gone("Switch Theme").settle()
    print("\n".join(tui.screen().split("\n")[-2:]))
    assert tui.colors_hash(1, (3, 30)) != base, (
        "Enter on the first picker row did not apply that theme"
    )
    tui.quit()
