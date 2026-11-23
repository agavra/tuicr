#!/usr/bin/env python3
# Runner self-test (expect PASS): the fake-forge log readers parse records: a partial trailing line is skipped, note
# records (HELD/RELEASED/UNEXPECTED) are not calls, since=forge_mark() hides earlier calls, regexes see the DECODED
# stdin (non-ASCII kept), and posts()/mutations() know every method spelling.
import json

from e2e.tui import Tui

from e2e import Failure, Scenario, must_fail, scenario

EXPECTED = "PASS"  # asserted by e2e/selftest/run.py


def rec(tool: str, argv: list[str], stdin: object = None, note: str | None = None) -> str:
    r: dict[str, object] = {"tool": tool, "argv": argv, "stdin": None if stdin is None else json.dumps(stdin)}
    if note:
        r["note"] = note
    return json.dumps(r) + "\n"


@scenario
def main(s: Scenario) -> None:
    tui = Tui(s, s.name + "_log")  # never started: the readers only need its log (not a checked forge log)
    tui.dir.mkdir(parents=True, exist_ok=True)
    log = tui.forge_log
    log.write_text(
        rec("gh", ["api", "repos/o/r/pulls/1/files"])
        + rec("gh", ["api", "repos/o/r/pulls/1/files"], note="HELD files.json")
        + rec("gh", ["api", "repos/o/r/pulls/1/files"], note="RELEASED files.json")
    )
    assert tui.forge_count("pulls/1/files") == 1, "a note record counted as a call"
    assert tui.forge_count("HELD", notes=True) == 1, "notes=True does not see the HELD note"
    must_fail(lambda: tui.wait_forge_log("pulls/1/files", 0.3, count=2), "HELD/RELEASED notes satisfied count=2")
    mark = tui.forge_mark()
    assert mark == 3, f"mark {mark}"
    must_fail(lambda: tui.forge_expect("pulls/1/files", since=mark), "since= saw a call made before the mark")
    with open(log, "a") as f:
        f.write(rec("gh", ["api", "--method", "POST", "repos/o/r/pulls/1/reviews", "--input", "-"], {"body": "日本 x"}))
        f.write(rec("gh", ["api", "-X", "POST", "repos/o/r/pulls/1/reviews", "--input", "-"], {"body": "b"}))
        f.write(rec("gh", ["api", "--method=PATCH", "repos/o/r/pulls/comments/5", "--input", "-"], {"body": "c"}))
        f.write(rec("glab", ["api", "-XDELETE", "projects/1/merge_requests/1/draft_notes/2"]))
        f.write('{"tool": "gh", "argv": ["api", "--meth')  # a fake still writing: skipped, not an error
    tui.forge_expect('"body": "日本 x"', since=mark)  # decoded stdin, non-ASCII kept
    must_fail(lambda: tui.forge_reject('"body": "b"'), "forge_reject missed a match in the decoded stdin")
    posts = tui.posts()
    assert [p["payload"]["body"] for p in posts] == ["日本 x", "b"], f"posts() missed a spelling: {posts}"
    assert tui.posts(since=tui.forge_mark()) == [], "posts(since=mark) saw earlier calls"
    methods = [c["method"] for c in tui.mutations()]
    assert methods == ["POST", "POST", "PATCH", "DELETE"], f"mutations(): {methods}"
    assert len(tui.forge_calls(where=lambda c: c["tool"] == "glab")) == 1
    with open(log, "a") as f:
        f.write("not json\n")  # a corrupt complete record fails loudly
    must_fail(tui.forge_calls, "a corrupt record was ignored", Failure)
