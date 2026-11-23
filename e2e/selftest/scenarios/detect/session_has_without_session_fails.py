#!/usr/bin/env python3
# Runner self-test (expect PASS): session_has() with no saved session at all raises (a `not session_has(...)` would
# otherwise pass without checking anything); allow_missing=True opts out. The parsed accessors fail too.
from e2e import Scenario, must_fail, scenario

EXPECTED = "PASS"  # asserted by e2e/selftest/run.py


@scenario
def main(s: Scenario) -> None:
    must_fail(lambda: s.session_has("anything"), "session_has passed with no saved session")
    assert not s.session_has("anything", allow_missing=True)
    must_fail(s.session_comments, "session_comments passed with no saved session")
    must_fail(lambda: s.session_reviewed("a.txt"), "session_reviewed passed with no saved session")
