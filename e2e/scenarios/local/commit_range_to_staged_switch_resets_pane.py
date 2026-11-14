#!/usr/bin/env python3
# Covers #896: switching from a commit-range review to "Staged changes" through :commits
#  (a) clears the range's commit pane (baseline kept it), and
#  (b) saves the outgoing range session (its reviewed mark) before replacing it; baseline lost it.
import json

from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.keys("r").wait_for("Files · 1/8")
    tui.cmd("commits").wait_for("Staged changes")
    tui.keys("j").keys("Enter")
    tui.wait_for(r"staged\.txt").settle(0.4)
    tui.reject("┌ Commits")  # stale commit pane kept after switching to a local target
    tui.quit()
    txt = s.session_json()
    dec = json.JSONDecoder()
    i = saved = 0
    while i < len(txt):
        while i < len(txt) and txt[i].isspace():
            i += 1
        if i >= len(txt):
            break
        o, i = dec.raw_decode(txt, i)
        if o.get("diff_source") == "commit_range" and any(f.get("reviewed") for f in o["files"].values()):
            saved += 1
    print(f"range sessions with a reviewed file on disk: {saved}")
    assert saved >= 1, "outgoing range session (reviewed mark) was not saved on the target switch"
