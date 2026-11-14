#!/usr/bin/env python3
# Covers #901: transparent_background (default on) must survive runtime theme changes. Baseline painted the opaque
# palette bg (light = 245;243;232) after `:theme light` and during picker previews.
from e2e import Scenario, Tui, scenario

OPAQUE_LIGHT_BG = "48;2;245;243;232"


def cnt(tui: Tui) -> int:
    return tui.screen(colors=True, join=False).count(OPAQUE_LIGHT_BG)


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    print(f"opaque light bg cells before: {cnt(tui)}")
    tui.cmd("theme light").settle(0.4)
    c = cnt(tui)
    print(f"opaque light panel_bg cells after :theme light: {c}")
    assert c == 0, (
        ":theme light painted an opaque panel background despite transparent_background"
    )
    tui.cmd("theme").wait_for("Switch Theme")
    tui.keys("j").settle().keys("k").settle().keys("j").settle()
    c = cnt(tui)
    print(f"after picker preview: {c}")
    assert c == 0, "picker preview painted an opaque panel background"
    tui.keys("Escape").quit()
