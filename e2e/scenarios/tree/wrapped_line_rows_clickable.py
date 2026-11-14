#!/usr/bin/env python3
# with wrap on, a wrapped line cut off by the viewport bottom is still drawn, so its rows must be clickable.
# Baseline's row->line map stopped before the partly visible line: clicking its visible rows did nothing.
from e2e import Scenario, because, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    repos.write(r / "base.txt", "base\n")
    repos.git(r, "add", "-A")
    repos.git(r, "commit", "-qm", "base")
    lines = [f"short line {i:02d}" for i in range(1, 37)]
    words = " ".join(f"word{i:03d}" for i in range(200))
    repos.write(r / "big.txt", "\n".join(lines) + "\n" + words + "\nlast\n")
    repos.git(r, "add", "big.txt")
    tui = s.start("-w", cwd=r)
    tui.wait_for(r"big.txt \[A\]").keys("Escape")
    tui.keys(":set wrap", "Enter").wait_for("word014")
    scr = tui.screen()
    cursor = "\n".join(line for line in scr.split("\n") if "▶" in line)
    assert grep(r"big.txt \[A\]", cursor), (
        f"precondition: cursor not on the header\n{scr}"
    )
    # click the last visible diff row (bottom of the cut-off wrapped line 37)
    tui.click(60, 43).settle()
    with because("click on the partly visible wrapped line did not select it"):
        tui.expect(r"▶ +37 +▌ word000")
    tui.quit()
