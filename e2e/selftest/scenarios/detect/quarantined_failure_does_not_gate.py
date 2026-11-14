#!/usr/bin/env python3
# Runner self-test (expect QFAIL via QUARANTINE below): a known failure is reported, not gating.
from e2e import Failure, Scenario, scenario

EXPECTED = "QFAIL"  # asserted by e2e/selftest/run.py
QUARANTINE = "selftest: demonstrates a tracked quarantine"


@scenario
def main(s: Scenario) -> None:
    raise Failure("selftest: a known failure")
