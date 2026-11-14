#!/usr/bin/env python3
# Runner self-test (expect PASS): with no pane at all, every screen/process query must FAIL. Before the refactor a
# missing pane counted as "text disappeared", "forbidden text absent" and "process exited" (all returned 0).
from e2e.tui import Tui

from e2e import NoPane, Scenario, must_fail, scenario


@scenario
def main(s: Scenario) -> None:
    tui = Tui(s, s.name)  # never started: there is no pane
    must_fail(lambda: tui.wait_for("anything", 1), "wait_for succeeded without a pane", NoPane)
    must_fail(lambda: tui.wait_gone("anything", 1), "wait_gone succeeded without a pane", NoPane)
    must_fail(lambda: tui.reject("anything", 1), "reject succeeded without a pane", NoPane)
    must_fail(lambda: tui.wait_exit(1), "wait_exit succeeded without a pane", NoPane)
    must_fail(lambda: tui.settle(0.2, 1), "settle succeeded without a pane", NoPane)
    must_fail(tui.screen, "screen succeeded without a pane", NoPane)
    must_fail(tui.alive, "alive is true without a pane", NoPane)
    assert not tui.pane_exists(), "pane_exists is true without a pane"
    print("all seven queries failed on a missing pane, as required")
