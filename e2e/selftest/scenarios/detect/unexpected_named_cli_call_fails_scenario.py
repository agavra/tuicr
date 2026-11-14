#!/usr/bin/env python3
# Runner self-test (expect FAIL): s.cli_result(name=...) logs to its own forge log; an unknown call there must fail too.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    s.cli_result("sh", "-c", "gh api repos/octo/demo/nonexistent >/dev/null 2>&1; exit 0", name="other")
