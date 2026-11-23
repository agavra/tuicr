#!/usr/bin/env python3
# `:set noreviewed` hides files marked reviewed (tree, diff); `r` then moves on to the next file; the tree title keeps
# counting them; a file whose hunk is only `R`-marked stays; `:set reviewed` brings them back.
from e2e import Scenario, because, scenario


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
    # data.bin left the tree; the next file is up: the diff pane is titled gone.txt
    tui.wait_gone(r"M data\.bin").expect(r"D gone\.txt")
    with because("`r` under :set noreviewed did not move on to the next file"):
        tui.wait_for(r"┌ gone\.txt ─")
    # a hunk-level `R` mark alone does not hide its file
    tui.cursor_on("delete me")  # onto a deleted line, inside gone.txt's only hunk
    tui.keys("R").wait_for("Hunk marked reviewed")
    tui.check(r"D gone\.txt", "an R-marked hunk hid its file").check("Files · 2/8", "R counted as a file review")
    tui.save()
    gone = s.session()["files"]["gone.txt"]
    assert gone["reviewed_hunks"] and not gone["reviewed"], f"R did not mark only the hunk of gone.txt: {gone}"
    tui.keys(":reviewed", "Enter")  # bare :reviewed toggles back to showing
    tui.wait_for(r"▣ M README\.md").reject("reviewed hidden")
    tui.quit()
