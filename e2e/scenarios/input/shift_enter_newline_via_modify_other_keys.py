#!/usr/bin/env python3
# Covers #782: Ghostty + tmux. tmux answers the kitty keyboard probe with DA1 only, so tuicr must ask for xterm
# modifyOtherKeys; tmux with `extended-keys on` and `extended-keys-format csi-u` then reports Shift-Enter as
# CSI 13;2u, and the comment gets a newline instead of being submitted.
from e2e import TUICR_BIN, Scenario, because, repos, scenario, ttyrelay, wait_cmd


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    log = s.dir / "relay.jsonl"
    tui = s.start_cmd(ttyrelay.argv(log, TUICR_BIN, "--no-update-check", "-r", "HEAD~2..HEAD", da1_only=True), cwd=repo)
    tui.wait_for("Files · 0/8").keys("Escape")

    # The probe must be answered like tmux, and tuicr must then request modifyOtherKeys. Without the request tmux
    # never sends the CSI-u sequence below, so this is the step that fails without the fallback.
    wait_cmd(10, lambda: "probe-answered" in ttyrelay.controls(log))
    with because("tuicr did not request xterm modifyOtherKeys on a DA1-only terminal (tmux)"):
        wait_cmd(10, lambda: "modify-other-keys:1" in ttyrelay.controls(log))

    tui.cursor_on("4 ▌ second line")
    tui.keys("c").wait_for("^ COMMENT ")
    tui.type("one")
    tui.type("\x1b[13;2u")  # what tmux sends for Shift-Enter with extended-keys-format csi-u
    tui.type("two").keys("Enter")
    tui.wait_saved_comment("one\ntwo")
    tui.quit()

    controls = ttyrelay.controls(log)
    assert "modify-other-keys:0" in controls, f"tuicr left modifyOtherKeys enabled after exit: {controls}"
