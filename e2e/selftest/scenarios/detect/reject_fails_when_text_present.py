#!/usr/bin/env python3
# Runner self-test (expect PASS): reject fails when the text is on screen, and its default window scales with
# E2E_TIMEOUT (0.04 * TIMEOUT unless E2E_REJECT_WINDOW says otherwise).
import os

from e2e.core import REJECT_WINDOW, TIMEOUT

from e2e import Scenario, must_fail, scenario

EXPECTED = "PASS"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    want = float(os.environ.get("E2E_REJECT_WINDOW") or 0.04 * TIMEOUT)
    assert want == REJECT_WINDOW, f"reject window {REJECT_WINDOW} is not 0.04 * E2E_TIMEOUT ({want})"
    tui = s.start_cmd(["sh", "-c", "echo late-arrival; sleep 60"])
    tui.wait_for("late-arrival", 5)
    must_fail(lambda: tui.reject("late-arrival"), "reject passed with the text on screen")
    tui.reject("never-shown")
