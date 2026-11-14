#!/usr/bin/env python3
# Covers #884: a parentless (root) Gerrit change diffs against the empty tree. Before, its base was an
# empty string, so the local `git diff <base>..<head>` failed and the change could not be opened.
from e2e import Failure, Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.gerrit_open("root")
    try:
        tui.wait_for("a.txt", 20)
    except Failure:
        log = s.dir / "gerrit" / "gerrit.log"
        print(log.read_text() if log.exists() else "")
        raise
    tui.settle().keys("}")
    tui.wait_for("line two")  # else: root change diff not shown
    tui.quit()
