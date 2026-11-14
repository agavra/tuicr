#!/usr/bin/env python3
# Covers #897: a Gerrit change URL copied from the browser (patch-set path, ?query, #message anchor) opens the change.
# Baseline rejects anything after the change number.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    for target in (
        "http://127.0.0.1:@PORT@/c/proj/+/1/1?tab=comments#message-abc",
        "http://127.0.0.1:@PORT@/c/proj/+/1#message-abc",
    ):
        # origin must name the same host as the URL
        tui = s.gerrit_open("root", host="127.0.0.1", target=target)
        tui.wait_for(r"a\.txt", 20)  # else: URL with patch set, query and fragment did not open the change
        tui.stop()
        s.gerrit_stop()
