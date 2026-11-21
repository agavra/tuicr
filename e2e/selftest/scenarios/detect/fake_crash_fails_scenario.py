#!/usr/bin/env python3
# Runner self-test (expect FAIL): a fake CLI that raises (here: its fixture files are missing) logs an UNEXPECTED note,
# so the scenario fails although the command's failure was swallowed.
from e2e import Scenario, scenario

EXPECTED = "FAIL"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    empty = s.dir / "no-data"
    empty.mkdir(parents=True, exist_ok=True)
    tui = s.start_cmd(["sh", "-c", "tea api -i /user >/dev/null 2>&1; sleep 30"], env={"E2E_DATA": str(empty)})
    tui.wait_forge_log("fake crashed", 10, notes=True)
