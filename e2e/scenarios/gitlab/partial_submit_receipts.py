#!/usr/bin/env python3
# Covers #898: GitLab `:submit draft` of two inline comments where the 2nd POST fails. The 1st landed on the forge;
# the retry must post only the 2nd. Baseline leaves both as local drafts and re-posts the 1st (duplicate on the forge).
import json

from e2e import Scenario, scenario, wait_grep


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    d.mkdir(parents=True, exist_ok=True)
    failflag, drafts = d / "failflag", d / "gl_drafts.json"
    failflag.touch()
    tui = s.forge_open("https://gitlab.com/octo/demo.git", env={"E2E_GL_FAIL_FLAG": str(failflag)})

    def posted() -> int:
        try:
            return len([x for x in json.loads(drafts.read_text()) if "first-note" in json.dumps(x)])
        except Exception:
            return 0

    tui.keys("}").cursor_on("1[╭ ]▌ pub fn greet")
    tui.keys("c").wait_for("COMMENT").keys("first-note").keys("Enter").wait_for("first-note")
    tui.cursor_on(r"2[╭ ]▌ +println")
    tui.keys("c").wait_for("COMMENT").keys("second-note").keys("Enter").wait_for("second-note")
    tui.cmd("submit draft").wait_for("Inline: 2").keys("y")
    # barrier: the 2nd POST failed on the forge AND the UI processed the failure (the 1st comment's receipt is recorded)
    tui.wait_forge_log('"note": "injected failure"', 15).wait_for("Submit failed", 15).settle()
    assert posted() == 1, "setup: first comment should have landed once"
    failflag.unlink()
    tui.cmd("submit draft").wait_for("Push pending|Submit review to")
    tui.keys("y")
    # barrier: the retry's POST landed on the forge
    wait_grep(drafts, "second-note", 15)
    tui.wait_for("Pushed pending GitLab review", 15)
    assert posted() == 1, f"retry re-posted the comment that already landed (first-note x{posted()})"
    assert "second-note" in drafts.read_text(), "retry did not post the failed comment"
    tui.quit()
