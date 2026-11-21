#!/usr/bin/env python3
# Runner self-test (expect FAIL): a POST to the fake Gerrit (it serves GETs only) is logged as UNEXPECTED.
import urllib.error
import urllib.request

from e2e import Scenario, scenario

EXPECTED = "FAIL"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    _, url = s.gerrit_serve("root")
    req = urllib.request.Request(url + "/changes/1/revisions/x/review", data=b"{}", method="POST")
    try:
        urllib.request.urlopen(req, timeout=5)
    except urllib.error.HTTPError as e:
        assert e.code == 405, e.code
