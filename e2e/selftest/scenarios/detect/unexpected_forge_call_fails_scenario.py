#!/usr/bin/env python3
# Runner self-test (expect FAIL): the scenario body "succeeds", but a fake forge saw a call it has no fixture for.
# tuicr treats some forge failures as best-effort, so only the central check can catch this.
from e2e import scenario


@scenario
def main(s):
    tui = s.start_cmd(["sh", "-c", "gh api repos/octo/demo/nonexistent >/dev/null 2>&1; sleep 30"])
    tui.wait_forge_log("UNEXPECTED", 10)
