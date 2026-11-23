#!/usr/bin/env python3
# Runner self-test (expect FAIL): an allowance covers only calls matching its pattern; an unexpected call outside it
# still fails the scenario.
from e2e import Scenario, scenario

EXPECTED = "FAIL"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    s.allow_unexpected_forge_calls(r"repos/octo/demo/nonexistent", 1, "the one call this scenario provokes")
    cmd = "gh api repos/octo/demo/nonexistent >/dev/null 2>&1; gh api repos/octo/demo/other >/dev/null 2>&1; sleep 30"
    tui = s.start_cmd(["sh", "-c", cmd])
    tui.wait_forge_log("UNEXPECTED", 10, count=2, notes=True)
