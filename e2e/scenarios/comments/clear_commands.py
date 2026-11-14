#!/usr/bin/env python3
# `:clearc` drops every comment but keeps reviewed marks; `:clear` drops the marks too.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("first remark").keys("Enter").wait_for("first remark")
    tui.keys("g").keys("r").wait_for("Files · 1/8")
    tui.cmd("clearc")
    tui.wait_gone("first remark").expect("Files · 1/8").expect(r"▣ M README\.md")
    tui.cursor_on(r"u32 \{ 2 \}")  # README.md is reviewed, so it is folded: comment elsewhere
    tui.keys("c").wait_for("COMMENT")
    tui.keys("second remark").keys("Enter").wait_for("second remark")
    tui.cmd("clear")
    tui.wait_gone("second remark").wait_for("Files · 0/8")
    tui.save()
    assert not s.session_has(r'remark|"reviewed": true'), f"cleared state was saved\n{s.session_json()}"
    tui.quit()
