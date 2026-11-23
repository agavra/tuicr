#!/usr/bin/env python3
# Regression from #493: Esc in the inline commit selector returns focus to the diff. #493 mapped Normal-mode Esc to
# "clear search highlight", so the selector's exit never ran and focus stayed on the commits. Space proves where
# focus is: on the selector it narrows the range to the cursor commit (a "Commit Message" pane and Files · 0/2).
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.open().wait_for("Files · 0/8")  # -r HEAD~2..HEAD, focus on the diff
    tui.keys(";k").settle()  # focus the commit selector
    tui.keys("Escape").settle()  # alone: Escape followed at once by Space reads as Alt-Space
    tui.keys("Space", "?").wait_for(r"Help \(j/k scroll")  # help open: Space was handled
    tui.keys("q").wait_gone(r"Help \(j/k scroll").settle()
    tui.reject(r"Commit Message \(").check("Files · 0/8", "Space after Esc changed the commit selection")
    # Back on the selector, Space still drives the commit selection.
    tui.keys(";k", "Space").wait_for(r"Commit Message \(").expect("Files · 0/2")
    tui.quit()
