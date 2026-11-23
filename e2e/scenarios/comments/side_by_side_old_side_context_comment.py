#!/usr/bin/env python3
# Covers #887: a context row exists on both sides of side-by-side; an old-side (left pane) comment
# on it must show its input box while typing and render after saving. Before, only New-side comments were handled
# on context rows: the input was invisible and the stored comment vanished.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cmd("diff")
    tui.wait_for("3  first line +│ +3  first line")
    tui.cursor_on("3  first line")
    tui.keys(";h").settle()
    tui.keys("c").wait_for("^ COMMENT ")
    tui.type("oldside note xyz")
    tui.wait_for("oldside note xyz")  # old-side comment input box visible
    tui.keys("Enter")
    tui.wait_saved_comment("oldside note xyz")
    tui.expect("oldside note xyz")  # saved old-side comment rendered on the context row (the editor is closed)
    stored = [c for f in s.session()["files"].values() for cs in f["line_comments"].values() for c in cs]
    assert [(c["content"], c["side"]) for c in stored] == [("oldside note xyz", "old")], (
        f"not stored on the old side: {stored}"
    )
    tui.quit()
