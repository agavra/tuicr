#!/usr/bin/env python3
# Covers the editor snapshot dir: `e` on a PR file the checkout lacks opens a read-only snapshot. It must live in a
# private
# (0700, user-owned) base directory named by the full head sha, not in a world-readable
# $TMPDIR/tuicr/snapshots/<repo>-pr<N>-<short>.
from e2e.forge import pr_editor_snapshot

from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    pr_editor_snapshot(s)
