#!/usr/bin/env python3
# Covers #888: file tree: horizontal scroll moves every row, including its indent and status glyph.
# Baseline pins each row's indent while only the name scrolls (so `an_extremely_...` keeps 4 leading spaces and `b.txt` 2).
from e2e import Scenario, because, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.wide_repo(s.dir / "repo")
    tui = s.start("-r", "HEAD~1..HEAD", cwd=repo)
    tui.wait_for("Files · 0/4").keys("Escape")
    tui.cmd("set nowrap").cmd("diff")
    # new side -> old side -> file list
    tui.keys(";h").settle(0.2).keys(";h").settle(0.2)
    tui.keys("l").settle()
    # scrolled by 4 columns: the nested file row now starts at its glyph/status, nothing pins the indent
    with because("file row not scrolled uniformly (indent pinned)"):
        tui.wait_for("^│▢ M an_extremely_long_file_nam")
    with because("b.txt row not scrolled uniformly"):
        tui.expect(r"^│M b\.txt ")
    tui.quit()
