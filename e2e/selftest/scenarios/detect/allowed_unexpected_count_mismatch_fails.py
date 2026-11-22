#!/usr/bin/env python3
# Runner self-test (expect FAIL): an allowance for 2 matching unexpected calls fails when only 1 is made.
from e2e import Scenario, scenario

EXPECTED = "FAIL"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    s.allow_unexpected_forge_calls(r"repos/octo/demo/nonexistent", 2, "two calls are declared, one is made")
    tui = s.start_cmd(["sh", "-c", "gh api repos/octo/demo/nonexistent >/dev/null 2>&1; sleep 30"])
    tui.wait_forge_log("UNEXPECTED", 10, notes=True)
