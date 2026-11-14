#!/usr/bin/env python3
# Runner self-test (expect QFAIL via the selftest quarantine file): a known failure is reported, not gating.
from e2e import Failure, Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    raise Failure("selftest: a known failure")
