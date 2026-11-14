#!/usr/bin/env python3
# `:N` (new side), `:oN` (old side) and `{N}G` put the cursor on that source line even inside collapsed context, expanding
# only as far as needed; `Enter` on an "expand" row reveals the hidden lines and `Enter` on revealed context folds them back.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    repos.write(r / "f.txt", "".join(f"row {i}\n" for i in range(1, 101)))
    repos.write(r / "g.txt", "x\n")
    repos.git(r, "add", "-A")
    repos.git(r, "commit", "-qm", "base")
    # three lines replace "row 5", so later rows are numbered +2 on the new side
    text = (r / "f.txt").read_text()
    text = text.replace("row 5\n", "ins a\nins b\nins c\n").replace(
        "row 70\n", "row 70 CHANGED\n"
    )
    repos.write(r / "f.txt", text)
    tui = s.start("-w", cwd=r)
    tui.wait_for("row 70 CHANGED").keys("Escape")
    tui.reject("row 28")  # hidden to begin with
    tui.cmd("30").wait_for("▶ +30 +row 28")  # new-side line 30 is "row 28"
    tui.cmd("o30").wait_for("▶ +32 +row 30")  # old-side line 30 is "row 30"
    tui.keys("5", "0", "G").wait_for("▶ +50 +row 48")
    # fresh view: Enter on an expand row reveals its lines, Enter on revealed context folds them again
    tui.cmd("e").wait_gone("row 48")
    tui.keys("g").cursor_on(r"expand \(20 lines\)")
    tui.keys("Enter").wait_for("row 28")
    tui.keys("Enter").wait_gone("row 28")
    tui.quit()
