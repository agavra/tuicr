#!/usr/bin/env python3
# Runner self-test (expect SKIP).
from e2e import scenario


@scenario
def main(s):
    s.skip("demonstrating a skip")
