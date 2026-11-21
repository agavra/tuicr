#!/usr/bin/env python3
# Edit (i) and delete (dd) a comment at the cursor: the edit replaces the stored text (it does not add a second
# comment or keep the old one), and the delete removes it from the rendered diff and the saved session.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.comment("original text")
    assert s.session_comments() == ["original text"], s.session_comments()
    # m jumps to the next comment (the cursor lands on the comment box)
    tui.keys("g").keys("m")
    tui.keys("i").wait_for("^ COMMENT ")
    tui.type(" EDITED").keys("Enter")
    tui.wait_saved_comment("original text EDITED")
    # the edit is what renders and what is stored, in place of the original
    tui.expect("original text EDITED")
    assert s.session_comments() == ["original text EDITED"], f"edit did not replace the comment: {s.session_comments()}"
    # delete
    tui.keys("d", "d")
    tui.wait_gone("original text")
    tui.save()
    assert s.session_comments() == [], f"deleted comment still in the session: {s.session_comments()}"
    tui.quit()
