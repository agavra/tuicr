#!/usr/bin/env python3
# Covers #897: a Gerrit change URL copied from the browser (patch-set path, ?query, #message anchor) opens the change.
# Baseline rejects anything after the change number. The change is 7 at patch set 3, so a parser that took the
# patch-set segment (or anything else) for the change number asks the fake Gerrit for an unknown change and fails.
from e2e.tui import read_forge_log

from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    for target in (
        "http://127.0.0.1:@PORT@/c/proj/+/7/3?tab=comments#message-abc",
        "http://127.0.0.1:@PORT@/c/proj/+/7#message-abc",
    ):
        # origin must name the same host as the URL
        tui = s.gerrit_open("root", host="127.0.0.1", target=target, change=7, patch_set=3)
        tui.wait_for(r"a\.txt", 20)  # else: URL with patch set, query and fragment did not open the change
        paths = {c["argv"][1].split("?")[0].rstrip("/") for c in read_forge_log(s.dir / "gerrit" / "forge.log")}
        assert "/changes/7" in paths, f"change 7 was never fetched: {paths}"
        tui.stop()
        s.gerrit_stop()
