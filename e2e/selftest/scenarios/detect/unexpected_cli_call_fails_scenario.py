#!/usr/bin/env python3
# Runner self-test (expect FAIL): a CLI-only scenario (no TUI) whose fake forge saw an unknown call must still fail.
# TUICR_BIN is `env` here, so s.cli runs the command itself.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    s.cli("sh", "-c", "gh api repos/octo/demo/nonexistent >/dev/null 2>&1; exit 0")
