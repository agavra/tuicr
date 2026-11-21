#!/usr/bin/env python3
# Covers #912: an external edit of the persisted session that clears a hunk's reviewed mark must be merged by the
# running TUI (three-way) instead of being overwritten by its next save. Baseline resurrected the hunk mark on `:w`.
import json
import os
from pathlib import Path

from e2e import Scenario, because, scenario


def hunks(f: Path) -> int:
    d = json.loads(f.read_text())
    return sum(len(v.get("reviewed_hunks") or []) for v in d["files"].values())


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.keys("R").wait_for(r"✓ @@")
    tui.save()
    files = s.session_files()
    assert files, "no saved session file"
    f = files[0]
    print(f"session: {f}")
    print(f"hunk marks after :w: {hunks(f)}")
    assert hunks(f) >= 1, "R did not persist a hunk mark"
    d = json.loads(f.read_text())
    for v in d["files"].values():
        v["reviewed_hunks"] = []
    tmp = Path(f"{f}.tmp")
    tmp.write_text(json.dumps(d))
    os.replace(tmp, f)
    # the running TUI polls the session file (review_watch_interval_ms = 1000) and unfolds the merged hunk
    with because("the TUI never merged the external edit (the hunk is still marked on screen)"):
        tui.wait_gone(r"✓ @@").wait_for("4 ▌ second line")
    tui.wait_gone("Saved to")  # the first save's message expires, so the next one is this save's
    tui.save()
    print(f"hunk marks after external clear + :w: {hunks(f)}")
    assert hunks(f) == 0, "TUI save resurrected the externally cleared hunk mark"
    tui.quit()
