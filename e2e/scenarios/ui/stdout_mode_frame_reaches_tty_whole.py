#!/usr/bin/env python3
# Covers #949: --stdout draws on /dev/tty, and a full repaint must reach the terminal in a few large writes, not one
# per cell. Written piecemeal, frames take long enough that zellij, which presents a pane only between synchronized
# updates, shows nothing while a mouse-wheel scroll keeps tuicr redrawing.
from e2e import TUICR_BIN, Scenario, repos, scenario, ttyrelay, wait_cmd


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    log, out = s.dir / "frames.jsonl", s.dir / "stdout.txt"
    tui = s.start_cmd(
        ttyrelay.argv(log, "sh", "-c", f"exec '{TUICR_BIN}' --no-update-check --stdout -r HEAD~2..HEAD >'{out}'"),
        cwd=repo,
    )
    tui.wait_for("Files · 0/8").keys("Escape").settle()
    before = len(ttyrelay.frames(log))
    tui.keys("C-l")  # rewrites every cell
    full = s.cols * s.rows // 2  # bytes: a repaint writes at least one per cell

    def repaint() -> dict[str, int] | None:
        return next((f for f in ttyrelay.frames(log)[before:] if f["bytes"] >= full), None)

    wait_cmd(10, repaint)
    frame = repaint()
    assert frame is not None
    per_read = frame["bytes"] / frame["reads"]
    assert per_read >= 256, (
        f"the repaint reached the terminal in {frame['reads']} reads of {frame['bytes']} bytes in total "
        f"({per_read:.0f} bytes each): tuicr wrote it in small pieces"
    )
    tui.quit()
