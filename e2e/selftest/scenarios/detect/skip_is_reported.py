#!/usr/bin/env python3
# Runner self-test (expect SKIP).
from e2e import Scenario, scenario

EXPECTED = "SKIP"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    s.skip("demonstrating a skip")
