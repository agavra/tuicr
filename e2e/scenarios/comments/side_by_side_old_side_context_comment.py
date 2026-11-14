#!/usr/bin/env python3
# Covers #887: a context row exists on both sides of side-by-side; an old-side (left pane) comment
# on it must show its input box while typing and render after saving. Before, only New-side comments were handled
# on context rows: the input was invisible and the stored comment vanished.
from e2e import Scenario, grep, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cmd("diff")
    tui.wait_for("3  first line +│ +3  first line")
    tui.cursor_on("3  first line")
    tui.keys(";h").settle()
    tui.keys("c").wait_for("COMMENT")
    tui.keys("oldside note xyz")
    tui.wait_for("oldside note xyz")  # old-side comment input box visible
    tui.keys("Enter").settle()
    tui.wait_for("oldside note xyz")  # saved old-side comment rendered on the context row
    assert s.session_has("oldside note xyz"), "comment not stored"
    assert grep(r'"side": *"old"', s.session_json()), f"comment not stored on the old side\n{s.session_json()[:3000]}"
    tui.quit()
