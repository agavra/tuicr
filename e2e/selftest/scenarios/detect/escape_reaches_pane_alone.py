#!/usr/bin/env python3
# Runner self-test (expect PASS): an Escape and the key sent after it reach the pane in separate reads. A terminal
# parser takes ESC followed by a key in one read as Alt+key (`Escape` then `r` became Alt-r and was lost). The reader
# below sleeps after each read, as a busy TUI would, so keys sent back to back pile up into one read.
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
    time.sleep(0.015)
"""


@scenario
def main(s: Scenario) -> None:
    reader = repos.write(s.dir / "reader.py", READER)
    tui = s.start_cmd(["python3", str(reader)])
    tui.wait_for("ready", 5)
    tui.keys("x").keys("Escape").keys("r").wait_for("72$", 5)
    tui.expect("^read 1b$").reject("1b72")
