#!/usr/bin/env python3
# Runner self-test (expect FAIL): a plain Python `assert` that is false must fail the scenario,
# with file:line in the log.
from e2e import Scenario, scenario

EXPECTED = "FAIL"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    assert 1 + 1 == 3, "arithmetic broke"
