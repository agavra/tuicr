#!/usr/bin/env python3
# Remote review threads: unresolved shown by default, :comments all adds resolved, hide removes, navigator lists them.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open()
    tui.expect("remote open thread: please reword").expect(r"\[github @bob\]")
    tui.reject("remote resolved thread", 1)
    tui.expect("Comments · 1")
    tui.cmd("comments all")
    tui.wait_for("remote resolved thread: fine now").expect("Comments · 2")
    tui.cmd("comments hide")
    tui.wait_gone("remote open thread").wait_gone("remote resolved thread")
    tui.cmd("comments unresolved")
    tui.wait_for("remote open thread").wait_gone("remote resolved thread")
    # remote threads are read-only: editing the thread at the cursor is refused
    tui.keys("m").settle().keys("i")
    tui.wait_for("read only|read-only")
    tui.quit()
