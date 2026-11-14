#!/usr/bin/env python3
# Add line / range / file / review comments; they render inline and persist with :w.
from typing import Any

from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.comment("line note alpha")
    tui.expect("── L4 ")
    # range comment over two rows (gone.txt deleted lines) via visual mode
    tui.cursor_on("1 ▌ delete me")
    tui.keys("v", "j").comment("range note beta")
    tui.expect("L1-L2|L1.{0,3}2")
    # file comment
    tui.keys("g").comment("file note gamma", key="C")
    # review-level comment
    tui.comment("review note delta", key=";c")
    # comment navigator sidebar lists them
    tui.expect("Comments")
    # persist, then verify on disk
    tui.save()
    # each comment is stored under its own anchor, not just somewhere in the file
    j = s.session()
    f = j["files"]

    def one(cs: list[dict[str, Any]], text: str) -> dict[str, Any]:
        assert len(cs) == 1 and cs[0]["content"] == text, (text, cs)
        return cs[0]

    line = one(f["README.md"]["line_comments"]["4"], "line note alpha")
    assert line["side"] == "new" and line["line_range"] is None, line
    rng = one(f["gone.txt"]["line_comments"]["2"], "range note beta")
    assert rng["side"] == "old" and rng["line_range"] == {"start": 1, "end": 2}, rng  # deleted lines: old side, L1-L2
    one(f["README.md"]["file_comments"], "file note gamma")
    one(j["review_comments"], "review note delta")
    # nothing leaked into another bucket
    assert sorted(c["content"] for fr in f.values() for cs in fr["line_comments"].values() for c in cs) == [
        "line note alpha",
        "range note beta",
    ], f
    assert sum(len(fr["file_comments"]) for fr in f.values()) == 1 and len(j["review_comments"]) == 1, j
    tui.quit()
