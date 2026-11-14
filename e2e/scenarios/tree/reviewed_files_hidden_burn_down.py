#!/usr/bin/env python3
# `:set noreviewed` hides files marked reviewed (tree, diff); `r` then moves on to the next file; the tree title keeps
# counting them; a file whose hunk is only `R`-marked stays; `:set reviewed` brings them back.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    # first file is reviewed (still shown)
    tui.keys("g").keys("r").wait_for("Files · 1/8")
    tui.expect(r"▣ M README\.md")  # reviewed rows carry ▣
    tui.cmd("set noreviewed")
    tui.wait_gone(r"▣ M README\.md")
    tui.wait_for("Files · 1/8")  # the progress fraction still counts the hidden file
    tui.wait_for("reviewed hidden")
    # burn-down: marking the file under the cursor hides it
    tui.keys("r").wait_for("Files · 2/8")
    # data.bin left the tree; the next file is up
    tui.wait_gone(r"M data\.bin").expect(r"D gone\.txt")
    # a hunk-level `R` mark alone does not hide its file
    tui.keys("R").settle().expect(r"D gone\.txt").wait_for("Files · 2/8")
    tui.keys(":reviewed", "Enter")  # bare :reviewed toggles back to showing
    tui.wait_for(r"▣ M README\.md").reject("reviewed hidden")
    tui.quit()
