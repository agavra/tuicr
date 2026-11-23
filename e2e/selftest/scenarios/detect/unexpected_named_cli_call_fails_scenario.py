#!/usr/bin/env python3
# Runner self-test (expect FAIL): s.cli_result(name=...) logs to its own forge log; an unknown call there must fail too.
from e2e import Scenario, scenario

EXPECTED = "FAIL"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    s.cli_result("sh", "-c", "gh api repos/octo/demo/nonexistent >/dev/null 2>&1; exit 0", name="other")
