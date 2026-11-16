#!/usr/bin/env python3
# Covers #884: a Gerrit thread's resolution comes from its LATEST reply. Thread 1 (open root, reply
# resolves it) must be hidden by the default unresolved-only filter; thread 2 (resolved root, reply reopens it)
# must show. Before, the root alone decided: the two were swapped.
from e2e import Failure, Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.gerrit_open("threads")
    try:
        tui.wait_for("a.txt", 20)
    except Failure:
        log = s.dir / "gerrit" / "gerrit.log"
        print(log.read_text() if log.exists() else "")
        raise
    tui.settle().keys("}")
    tui.wait_for("a.txt:2")  # else: reopened thread (line 2) not shown
    tui.settle()
    scr = tui.screen()
    assert "a.txt:1" not in scr, f"resolved-by-reply thread (line 1) is shown\n{scr}"
    tui.quit()
