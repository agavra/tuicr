#!/usr/bin/env python3
# <leader>p opens the file picker; / filters; Enter jumps; Esc closes without moving.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.keys(";p").wait_for(r"Go to File \(8/8\)")
    tui.keys("/").keys("tabs")
    tui.wait_for(r"Go to File \(1/8\)")
    tui.keys("Enter").wait_gone("Go to File").wait_for(r"┌ tabs\.txt ─")
    # Esc closes without moving
    tui.keys(";p").wait_for(r"Go to File \(8/8\)").keys("j", "Escape")
    tui.wait_gone("Go to File").expect(r"┌ tabs\.txt ─")
    # j then Enter jumps to the selected file
    tui.keys(";p").wait_for("Go to File").keys("j", "Enter")
    tui.wait_gone("Go to File").wait_for(r"┌ src/lib\.rs ─")
    tui.quit()
