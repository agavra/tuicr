#!/usr/bin/env python3
# Runner self-test (expect SKIP).
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    s.skip("demonstrating a skip")
