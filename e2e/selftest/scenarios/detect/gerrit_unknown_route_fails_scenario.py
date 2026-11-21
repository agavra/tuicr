#!/usr/bin/env python3
# Runner self-test (expect FAIL): the fake Gerrit logs an unknown route as UNEXPECTED (gerrit/forge.log is a checked
# forge log), so the scenario fails although the 404 was "handled".
import urllib.error
import urllib.request

from e2e import Scenario, scenario

EXPECTED = "FAIL"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    _, url = s.gerrit_serve("root")
    try:
        urllib.request.urlopen(url + "/changes/1/drafts", timeout=5)
    except urllib.error.HTTPError as e:
        assert e.code == 404, e.code
