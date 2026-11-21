#!/usr/bin/env python3
# Runner self-test (expect PASS): a pane whose process exited (remain-on-exit keeps it) must fail every screen wait and
# check: after a crash the screen is frozen, so "text gone", "forbidden text absent", "settled" and "changed" prove
# nothing. allow_dead=True, or a wait_exit that accepted the exit, lifts it.
from e2e import DeadPane, Scenario, must_fail, scenario

EXPECTED = "PASS"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    tui = s.start_cmd(["sh", "-c", "echo before-exit; sleep 0.5; exit 7"])
    tui.wait_for("before-exit", 5)
    before = tui.screen()
    for _ in range(50):  # wait for the death without wait_exit (which would accept it)
        if not tui.alive():
            break
        tui.settle(0.1, allow_dead=True)
    assert not tui.alive(), "the process never exited"
    must_fail(lambda: tui.wait_for("before-exit|Pane is dead", 1), "wait_for passed on a dead pane", DeadPane)
    must_fail(lambda: tui.expect("anything", 1), "expect passed on a dead pane", DeadPane)
    must_fail(lambda: tui.wait_gone("not-on-screen", 1), "wait_gone passed on a dead pane", DeadPane)
    must_fail(lambda: tui.reject("not-on-screen", 0.2), "reject passed on a dead pane", DeadPane)
    must_fail(lambda: tui.check("before-exit|Pane is dead", "check"), "check passed on a dead pane", DeadPane)
    must_fail(lambda: tui.settle(0.1, 1), "settle passed on a dead pane", DeadPane)
    must_fail(lambda: tui.wait_screen_change(before, 1), "wait_screen_change passed on a dead pane", DeadPane)
    must_fail(lambda: tui.cursor_on("x", 2), "cursor_on passed on a dead pane", DeadPane)
    tui.reject("not-on-screen", 0.2, allow_dead=True)  # the escape hatch works
    tui.wait_exit(5, 7)  # accepting the exit: the final screen can be checked now
    tui.wait_gone("not-on-screen", 1)
    print("every screen wait failed on a dead pane, as required")
