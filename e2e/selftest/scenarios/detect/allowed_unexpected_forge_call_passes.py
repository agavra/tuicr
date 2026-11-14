#!/usr/bin/env python3
# Runner self-test (expect PASS): a scenario that provokes unexpected calls on purpose declares it.
from e2e import scenario


@scenario
def main(s):
    s.allow_unexpected_forge_calls("the fake gh has no fixture for this route, on purpose")
    tui = s.start_cmd(["sh", "-c", "gh api repos/octo/demo/nonexistent >/dev/null 2>&1; sleep 30"])
    tui.wait_forge_log("UNEXPECTED", 10)
