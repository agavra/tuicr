#!/usr/bin/env python3
# File tree basics with the tree focused (<leader>h): j/k move the selection and Enter jumps the diff to the selected file;
# Space or Enter toggles the directory under the selection, O collapses and o expands every directory; child rows appear and
# disappear accordingly. Enter on a file moves focus to the diff, so each step refocuses the tree first.
from e2e import Scenario, Tui, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    row = r"▢ M lib\.rs"  # src/lib.rs as a tree row (the diff only shows the full path)

    def cur(name: str) -> None:
        # the diff pane title names the file the diff is on
        tui.wait_for(f"┌ {name} ─+ ")

    def tree() -> Tui:
        return tui.keys(";h").settle()

    tree().keys("Enter")
    cur(r"README\.md")  # selection starts on the first file
    tree().keys("j", "j", "Enter")
    cur(r"gone\.txt")  # data.bin, gone.txt
    tree().keys("k", "Enter")
    cur(r"data\.bin")
    # gone, new_name, notes_cjk, tabs, then the src/ directory row
    tree().keys("j", "j", "j", "j", "j").expect(row)
    # Space on the directory collapses it, Space again expands it; Enter on it toggles too and does not leave the tree
    tui.keys("Space").wait_gone(row).expect("▶ src/")
    tui.keys("Space").wait_for(row).expect("▼ src/")
    tui.keys("Enter").wait_gone(row).expect("▶ src/")
    tui.keys("Enter").wait_for(row).expect("▼ src/")
    cur(r"data\.bin")
    # a child is reachable with j + Enter
    tui.keys("j", "Enter")
    cur(r"src/lib\.rs")
    # O collapses every directory, o expands them all again
    tree().keys("O").wait_gone(row).wait_gone(r"▢ M main\.rs").expect("▶ src/")
    tui.keys("o").wait_for(row).expect(r"▢ M main\.rs")
    tui.quit()
