#!/usr/bin/env python3
# Runner self-test (expect QFAIL via the selftest quarantine file): a known failure is reported, not gating.
from e2e import Failure, scenario


@scenario
def main(s):
    raise Failure("selftest: a known failure")
