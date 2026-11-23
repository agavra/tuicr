#!/usr/bin/env python3
# GoToTop (g) skips filtered-out diff_files[0] in both continuous and single-file views.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.keys(";h").settle()
    tui.keys("i", "^src/", "Enter")
    tui.wait_for("Files · 0/2").wait_gone(r"README\.md")
    tui.keys(";l").settle()
    # Continuous view: g must land on the first visible src/ file, not hidden file 0.
    tui.keys("g", "r").wait_for("Files · 1/2")
    # Single-file view: g must render a visible src/ file, not a blank hidden file 0.
    tui.cmd("focus").settle()
    tui.keys("g").wait_for(r"┌ src/")
    tui.quit()
