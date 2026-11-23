#!/usr/bin/env python3
# `:N` into a line hidden inside a collapsed (reviewed) file must not expand the gap behind the
# user's back. Baseline expanded context rows 8..30 of the gap; once the file is un-reviewed they show up.
from e2e import Scenario, because, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    repos.write(r / "f.txt", "".join(f"row {i}\n" for i in range(1, 101)))
    repos.write(r / "g.txt", "x\n")
    repos.git(r, "add", "-A")
    repos.git(r, "commit", "-qm", "base")
    text = (r / "f.txt").read_text().replace("row 5\n", "row 5 CHANGED\n").replace("row 60\n", "row 60 CHANGED\n")
    repos.write(r / "f.txt", text)
    repos.write(r / "g.txt", "y\n")
    tui = s.start("-w", cwd=r)
    tui.wait_for("f.txt").keys("Escape")
    tui.keys("r").wait_for("Files · 1/2")  # f.txt reviewed => collapsed
    # the jump is handled: it reports line 30 as not shown (an expanded gap would show it and say nothing)
    with because("`:30` in the collapsed file was ignored or expanded the gap to reach the line"):
        tui.keys(":30", "Enter").wait_for("Line 30 (not in diff|not found)")
    tui.keys("r").wait_for("Files · 0/2")  # un-review, f.txt is rendered again
    tui.wait_for("row 5 CHANGED")
    tui.settle()
    scr = tui.screen()
    assert not grep(r"row (20|30)( |$)", scr), f"gap was expanded while the file was collapsed\n{scr}"
    assert grep("lines hidden", scr), f"expected a still-collapsed gap\n{scr}"
    tui.quit()
