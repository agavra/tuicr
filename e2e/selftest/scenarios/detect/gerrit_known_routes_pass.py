#!/usr/bin/env python3
# Runner self-test (expect PASS): control for the Gerrit UNEXPECTED checks: the routes the fake serves pass, and every
# request lands in gerrit/forge.log as a call.
import urllib.request

from e2e.tui import read_forge_log

from e2e import Scenario, scenario

EXPECTED = "PASS"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    _, url = s.gerrit_serve("root")
    for path in ("/changes/1/?o=CURRENT_REVISION", "/changes/1/comments"):
        with urllib.request.urlopen(url + path, timeout=5) as r:
            assert r.status == 200, r.status
    calls = [c["argv"] for c in read_forge_log(s.dir / "gerrit" / "forge.log")]
    assert calls == [["GET", "/changes/1/?o=CURRENT_REVISION"], ["GET", "/changes/1/comments"]], calls
