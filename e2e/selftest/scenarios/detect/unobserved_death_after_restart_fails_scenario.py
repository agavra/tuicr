#!/usr/bin/env python3
# Runner self-test (expect FAIL): restarting a TUI under the same name must not hide that the first process had died
# unobserved (the restart kills its pane).
from e2e import Scenario, scenario, wait_cmd

EXPECTED = "FAIL"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    first = s.start_cmd(["sh", "-c", "echo first; sleep 0.3; exit 4"])
    first.wait_for("first", 5, allow_dead=True)
    wait_cmd(10, lambda: not first.alive())  # dead, but no wait_exit looked at its status
    second = s.start_cmd(["sh", "-c", "echo second; sleep 60"], keep=True)
    second.wait_for("second", 5)
