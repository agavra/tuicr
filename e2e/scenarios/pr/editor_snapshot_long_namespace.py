#!/usr/bin/env python3
# Covers the snapshot cache dir name limit: with a 260-char owner the old `<slug>-pr1-<short>` dir exceeded NAME_MAX,
# so `e`
# failed to open the snapshot at all. Same private-snapshot checks as editor_snapshot_private_dir.
from e2e.forge import pr_editor_snapshot

from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    pr_editor_snapshot(s, f"https://github.com/{'n' * 260}/demo.git")
