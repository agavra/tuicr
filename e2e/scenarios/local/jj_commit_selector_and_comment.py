#!/usr/bin/env python3
# jj backend: the selector lists jj commits (working copy first); Enter reviews the working-copy change with its files
# and hunks; a comment saves under the jj session. Needs real jj (SKIP without it, FAIL with E2E_REQUIRE_JJ=1).
import json
from pathlib import Path

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    s.use_jj()
    repo = repos.jj_repo(s.dir / "repo")
    tui = s.start(cwd=repo)
    tui.wait_for("wip").expect("second").expect("base").expect("jj:")
    tui.keys("Enter")
    tui.wait_for("Files · 0/3").expect(r"M a\.txt").expect(r"A d\.txt").expect("a wip").expect("d wip")
    tui.reject("c one")  # c.txt belongs to "second", not to the working copy
    tui.keys("Escape").cursor_on("a wip")
    tui.comment("jj note")
    tui.save()
    # startup also saves an empty working-copy session; the comment lands in the reviewed change's session only
    found = [json.loads(f.read_text()) for f in s.session_files()]
    with_notes = [o for o in found if "jj note" in json.dumps(o)]
    assert len(with_notes) == 1, f"sessions holding the comment: {len(with_notes)} of {len(found)}"
    sess = with_notes[0]
    assert sess["diff_source"] == "commit_range", f"the comment was saved under {sess['diff_source']}"
    assert Path(sess["repo_path"]).resolve() == repo.resolve(), f"the session belongs to {sess['repo_path']}"
    notes = [c["content"] for cs in sess["files"]["a.txt"]["line_comments"].values() for c in cs]
    assert notes == ["jj note"], f"a.txt's line comments: {notes}"
    tui.quit()
