#!/usr/bin/env python3
# `:clearc` drops every comment but keeps reviewed marks; `:clear` drops the marks too.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.comment("first remark")
    tui.keys("g").keys("r").wait_for("Files · 1/8")
    tui.cmd("clearc")
    tui.wait_gone("first remark").expect("Files · 1/8").expect(r"▣ M README\.md")
    tui.save()
    assert s.session_comments() == [], f":clearc left comments in the session: {s.session_comments()}"
    assert s.session_reviewed("README.md"), ":clearc dropped the reviewed mark from the session"
    tui.cursor_on(r"u32 \{ 2 \}")  # README.md is reviewed, so it is folded: comment elsewhere
    tui.comment("second remark")
    tui.cmd("clear")
    tui.wait_gone("second remark").wait_for("Files · 0/8")
    tui.save()
    files = s.session()["files"]
    assert s.session_comments() == [], f"cleared comments were saved: {s.session_comments()}"
    assert not [p for p, f in files.items() if f["reviewed"]], f"cleared reviewed marks were saved: {files}"
    tui.quit()
