#!/usr/bin/env python3
# Covers #905: scroll the help to its end in a small terminal, then enlarge the terminal. The scroll offset
# must be clamped to the new (smaller) maximum; baseline keeps the old offset and the popup shows a blank tail.
from e2e import Scenario, grep, scenario


@scenario
def main(s: Scenario) -> None:
    s.rows = 20
    tui = s.diff()
    tui.keys("?").wait_for("HELP")
    tui.reject("Toggle this help")  # the help's last line is below the fold in 20 rows
    tui.keys("G").wait_for("Toggle this help")  # G reached the end
    tui.resize(160, 70).settle(0.4)
    lines = tui.screen().split("\n")

    # the popup must not end with a run of blank rows: count blank interior rows right above its bottom border
    last = next((i for i, ln in enumerate(lines, 1) if grep(r"└─{90,}┘ +│$", ln)), 0)
    assert last, "popup bottom border not found after resize\n" + "\n".join(lines)
    blank = 0
    i = last - 1
    while i > 0:
        # popup interior blank = only box-drawing chars/spaces between the popup's borders
        if grep(r"│ {90,}│", lines[i - 1]):
            blank += 1
            i -= 1
        else:
            break
    assert blank <= 3, f"help popup shows a blank tail after resize ({blank} rows)"
    tui.check("Toggle this help", "the help's last line left the view after the resize")
    tui.keys("q").wait_gone("Toggle this help")  # close the help first: it would take the `q` of `:q!`
    tui.quit()
