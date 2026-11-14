#!/usr/bin/env python3
# Runner self-test (expect PASS): wait_exit verifies the exit status (default 0); a crash is not "exited fine".
from e2e import Scenario, must_fail, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.start_cmd(["sh", "-c", "exit 3"])
    must_fail(lambda: tui.wait_exit(5), "wait_exit accepted status 3 as success")
    tui.wait_exit(5, 3)  # the expected status passes
    tui.wait_exit(5, "any")
    ok = s.start_cmd(["sh", "-c", "exit 0"], name=s.name + "_ok")
    ok.wait_exit(5)
