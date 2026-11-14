#!/usr/bin/env python3
# :summary lists pending local comments; Enter jumps to one; Esc returns.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("summary one").keys("Enter").wait_for("summary one")
    tui.keys("G").keys("C").wait_for("COMMENT")
    tui.keys("summary two").keys("Enter").wait_for("summary two")
    tui.cmd("summary")
    tui.wait_for("SUMMARY").expect("summary one").expect("summary two")
    tui.keys("j", "Enter")
    tui.wait_gone("SUMMARY")
    tui.expect("summary (one|two)")
    tui.cmd("summary").wait_for("SUMMARY")
    tui.keys("Escape").wait_gone("SUMMARY")
    tui.quit()
