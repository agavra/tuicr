#!/usr/bin/env python3
# Runner self-test (expect PASS when SETTLE_EXPECT matches): Tui.settle's default quiet period is the configured one
# ([tool.tuicr-e2e] settle_quiet, or the E2E_SETTLE_QUIET override), not a hard-coded value.
import inspect
import os

from e2e import Scenario, Tui, scenario

EXPECTED = "PASS"  # asserted by e2e/selftest/run.py (which runs this dir separately)


@scenario
def main(s: Scenario) -> None:
    default = inspect.signature(Tui.settle).parameters["quiet"].default
    assert default == float(os.environ["SETTLE_EXPECT"]), f"settle default {default}"
