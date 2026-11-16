#!/usr/bin/env python3
# Covers #893: head moves once while PR #1 opens (fake gh: head 2222.. until the first data call, 4444.. after).
# Fixed: the open re-reads the head, sees it changed, retries once and settles on 4444.. (session keyed to it).
# Baseline: opens silently pairing the old head's metadata with the newer diff, session keyed to 2222...
import re

from e2e import Scenario, pr_comment, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open(env={"E2E_HEAD_MODE": "once"})
    pr_comment(tui, "SNAPSHOT-DRAFT")
    tui.save()
    j = s.session_json()
    print(sorted(set(re.findall(r'"head_sha": *"[0-9a-f]*"', j))))
    assert re.search(r'"head_sha": *"4{40}"', j), "session not keyed to the settled head"
    assert not re.search(r'"head_sha": *"2{40}"', j), "session pinned to the stale head"
    tui.quit()
