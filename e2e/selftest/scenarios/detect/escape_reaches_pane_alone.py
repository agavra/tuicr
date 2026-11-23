#!/usr/bin/env python3
# Runner self-test (expect PASS): the key sent after an Escape never reaches the pane in the same read as the Escape.
# A terminal parser takes ESC followed by a key in one read as Alt+key (`Escape` then `r` became Alt-r and was lost).
# The reader below sleeps after each read far longer than any fixed gap, as a busy TUI would, so input that does not
# wait for the Escape to be read piles up behind it. Bytes before the Escape may share its read (a Linux pty can
# deliver `x` and ESC sent back to back together): only an Escape that ends its read counts.
from e2e import Scenario, repos, scenario

EXPECTED = "PASS"  # asserted by e2e/selftest/run.py

READER = """
import os, sys, time, tty
tty.setraw(0)
sys.stdout.write("ready\\r\\n")
sys.stdout.flush()
while True:
    sys.stdout.write("read " + os.read(0, 1024).hex() + "\\r\\n")
    sys.stdout.flush()
    time.sleep(0.3)
"""


@scenario
def main(s: Scenario) -> None:
    reader = repos.write(s.dir / "reader.py", READER)
    tui = s.start_cmd(["python3", str(reader)])
    tui.wait_for("ready", 5)
    tui.keys("x").keys("Escape").keys("r").wait_for("72$", 5)
    tui.reject("1b72", 0.1).wait_for("^read (78)?1b$", 0.1)
