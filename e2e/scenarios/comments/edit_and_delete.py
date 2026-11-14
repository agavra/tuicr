#!/usr/bin/env python3
# Edit (i) and delete (dd) a comment at the cursor.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("original text").keys("Enter")
    tui.wait_for("original text")
    # m jumps to the next comment (the cursor lands on the comment box)
    tui.keys("g").keys("m")
    tui.keys("i").wait_for("COMMENT")
    tui.keys(" EDITED").keys("Enter")
    tui.wait_for("original text EDITED")
    # delete
    tui.keys("d", "d")
    tui.wait_gone("original text EDITED")
    tui.save()
    assert s.session_json(), "no saved session to check"
    assert not s.session_has("original text"), f"deleted comment still in session json\n{s.session_json()}"
    tui.quit()
