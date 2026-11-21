#!/usr/bin/env python3
# Runner self-test (expect PASS): cursor_on/cursor_row read the diff cursor (▶ outside the `┌ Files` box), never the
# file tree's ▶ (collapsed directories) on the same screen row.
from e2e import Scenario, must_fail, repos, scenario

EXPECTED = "PASS"  # asserted by e2e/selftest/run.py

SCREEN = (
    "┌ Files ───┐┌ diff ──────────┐\n"
    "│▶ target/ ││   1 ▌ target   │\n"
    "│  other   ││▶  2 ▌ second   │\n"
    "└──────────┘└────────────────┘\n"
)


@scenario
def main(s: Scenario) -> None:
    screen = repos.write(s.dir / "screen.txt", SCREEN)
    tui = s.start_cmd(["sh", "-c", f'cat "{screen}"; sleep 60'])
    tui.wait_for("second", 5)
    assert "second" in tui.cursor_row() and "target" not in tui.cursor_row(), tui.cursor_row()
    tui.cursor_on("2 ▌ second", 1)
    must_fail(lambda: tui.cursor_on("target", 2), "cursor_on accepted the file tree's ▶ as the diff cursor")
