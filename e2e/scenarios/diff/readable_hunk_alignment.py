#!/usr/bin/env python3
# Covers #950: readable libgit2 defaults keep reordered functions and inserted blocks whole.
from pathlib import Path

from e2e import Scenario, repos, scenario

FIXTURES = Path(__file__).resolve().parents[3] / "tests/fixtures/diff_alignment"


@scenario
def main(s: Scenario) -> None:
    repo = repos.empty_repo(s.dir / "repo")
    cases = (
        ("reordered_functions", r"1   fn second\(\)", r"7○▌ fn first\(\)", 7),
        ("inserted_block", r"6       if ready", r"2○▌     if ready", 2),
    )
    for name, _, _, _ in cases:
        repos.write(repo / f"{name}.rs", (FIXTURES / f"{name}.old.rs").read_text())
    repos.git(repo, "add", "reordered_functions.rs", "inserted_block.rs")
    repos.git(repo, "commit", "-qm", "base")
    for name, _, _, _ in cases:
        repos.write(repo / f"{name}.rs", (FIXTURES / f"{name}.new.rs").read_text())

    for name, context, changed, line in cases:
        tui = s.start("-w", "-p", f"{name}.rs", cwd=repo)
        tui.wait_for(context).wait_for(changed)
        assert not s.session()["files"][f"{name}.rs"]["line_comments"], "precondition: no comments"
        tui.cursor_on(changed).comment(f"aligned {name}")
        tui.wait_for(f"aligned {name}")
        tui.save()
        comments = s.session()["files"][f"{name}.rs"]["line_comments"]
        assert list(comments) == [str(line)], comments
        assert len(comments[str(line)]) == 1, comments
        comment = comments[str(line)][0]
        assert comment["side"] == "new" and comment["content"] == f"aligned {name}", comment
        tui.quit()
