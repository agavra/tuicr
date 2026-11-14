#!/usr/bin/env python3
# a `:e` reload of PR #1 that is still in flight when the user opens PR #2 must be dropped.
# Before, its late result was applied to the PR now on screen (PR #2 showed PR #1's files).
import shutil

from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    d.mkdir(parents=True, exist_ok=True)
    variant, hold = d / "variant", d / "hold"
    variant.write_text("threads_and_commits\n")
    shutil.rmtree(hold, ignore_errors=True)
    hold.mkdir()
    tui = s.pr_open(env={"E2E_VARIANT_FILE": str(variant), "E2E_HOLD_DIR": str(hold)})
    tui.wait_for("PR #1")
    # start a reload and hold its data fetch (files/commits/diff of PR #1)
    (hold / "gate").touch()
    tui.cmd("e")
    # barrier: all three data calls (files, commits, diff) of the reload are parked on the forge
    tui.wait_forge_log('"note": "HELD', 10, count=3)  # else: reload data calls never reached the forge hold
    (hold / "gate").unlink()
    variant.write_text("second_pr\n")
    # open PR #2 from the selector while the reload is parked
    tui.cmd("prs").wait_for("Second PR other change", 10)
    tui.keys("j").settle().keys("Enter")
    tui.wait_for("PR #2|octo/demo#2", 15).wait_for(r"other\.rs", 10)
    # now let PR #1's reload finish
    (hold / "release").touch()
    # barrier: the three held data calls of PR #1's reload were served; then the UI absorbs (and must drop) the result
    tui.wait_forge_log('"note": "RELEASED', 10, count=3)  # else: held reload calls were never released
    tui.settle(0.4)
    scr = tui.screen()
    assert "octo/demo#2" in scr, f"PR #2 is no longer on screen\n{scr}"
    assert "greet.rs" not in scr, f"PR #1's late reload result replaced PR #2's files\n{scr}"
    assert "other.rs" in scr, f"PR #2's file list lost\n{scr}"
    tui.quit()
