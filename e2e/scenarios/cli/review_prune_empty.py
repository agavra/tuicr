#!/usr/bin/env python3
# Covers #407: prune removes empty saved reviews and leaves reviews with comments.
import json
from pathlib import Path
from typing import Any

from e2e import Scenario, scenario, wait_cmd


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    repo = s.dir / "repo"
    tui.cursor_on(r"second line").comment("keep this review")
    tui.quit()

    def sessions() -> list[dict[str, Any]]:
        return json.loads(s.cli("review", "list", cwd=repo))

    saved = sessions()
    assert len(saved) == 1 and saved[0]["comment_count"] == 1, saved
    kept = saved[0]["path"]

    tui = s.start("-w", cwd=repo, keep=True).wait_for("unstaged line")
    empty = [row for row in sessions() if row["active"] and row["comment_count"] == 0]
    assert len(empty) == 1, empty
    stale = empty[0]["path"]
    # A clean exit removes empty sessions. Simulate an interrupted review to leave one on disk.
    tui.kill()
    wait_cmd(10, lambda: any(row["path"] == stale and not row["active"] for row in sessions()))

    s.cli("review", "prune", "--empty", cwd=repo)
    assert not Path(stale).exists(), "prune left the empty session file"
    remaining = sessions()
    assert [(row["path"], row["comment_count"]) for row in remaining] == [(kept, 1)], remaining
    comments = json.loads(s.cli("review", "comments", "--session", kept, cwd=repo))
    assert [comment["content"] for comment in comments] == ["keep this review"], comments
