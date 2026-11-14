#!/usr/bin/env python3
# Runner self-test (expect FAIL): text that is not on screen is a failure, with the screen in the log.
from e2e import scenario


@scenario
def main(s):
    tui = s.start_cmd(["sh", "-c", "echo hello; sleep 60"])
    tui.wait_for("hello", 5)
    tui.expect("goodbye", 1)
