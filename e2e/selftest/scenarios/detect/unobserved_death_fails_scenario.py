#!/usr/bin/env python3
# Runner self-test (expect FAIL): a TUI that exits (here with status 3) while the scenario never calls wait_exit fails
# the scenario at teardown, even though the body itself "passed".
from e2e import Scenario, scenario, wait_cmd

EXPECTED = "FAIL"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    tui = s.start_cmd(["sh", "-c", "echo crashing; sleep 0.3; exit 3"])
    tui.wait_for("crashing", 5, allow_dead=True)
    wait_cmd(10, lambda: not tui.alive())  # dead, but no wait_exit looked at its status
