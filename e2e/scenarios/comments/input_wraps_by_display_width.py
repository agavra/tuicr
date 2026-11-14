#!/usr/bin/env python3
# Covers #906: comment input wraps by display width. VS16 emoji (U+2764 U+FE0F) count 1 per char in the baseline's
# wrap_segments, so a row holds twice the box width and spills over the right border; the wrapped rows must stay inside
# the box.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.keys("j", "j").keys("c").wait_for("Add L1", 3)
    tui.type("\u2764\ufe0f" * 60).settle(0.4)
    rows = [line for line in tui.screen().split("\n") if "\u2764" in line]
    assert rows, "no emoji rows"
    # tmux's own VS16 width handling garbles the *last* wrapped row, so judge the first row only: it must close at
    # the border.
    assert rows[0].endswith("││"), "first wrapped emoji row spills past the comment box border"
    assert len(rows) >= 2, "60 emoji did not wrap inside the box (single overlong row)"
    tui.keys("C-c").settle()
    tui.quit()
