#!/usr/bin/env python3
# Covers #892: a file-level inline comment (inline.path, no from/to) and a live reply to a deleted root
# comment are kept. Baseline: the file-level thread and the reply (with its deleted root) are dropped.
from e2e import Scenario, scenario

COMMENTS = """\
[{"id":1,"content":{"raw":"BB-FILE-LEVEL-NOTE"},"user":{"display_name":"carol"},"parent":null,"inline":{"path":"src/greet.rs","from":null,"to":null}},
 {"id":2,"content":{"raw":"gone"},"user":{"display_name":"dave"},"parent":null,"deleted":true,"inline":{"path":"src/greet.rs","from":null,"to":1}},
 {"id":3,"content":{"raw":"BB-REPLY-TO-DELETED"},"user":{"display_name":"erin"},"parent":{"id":2},"inline":{"path":"src/greet.rs","from":null,"to":1}}]
"""


@scenario
def main(s: Scenario) -> None:
    s.dir.mkdir(parents=True, exist_ok=True)
    comments = s.dir / "comments.json"
    comments.write_text(COMMENTS)
    tui = s.forge_open("https://bitbucket.org/octo/demo.git", env={"E2E_BB_COMMENTS": str(comments)})
    # the file-level note renders in the PR description pane (top); check before `}` scrolls it away
    tui.wait_for("BB-FILE-LEVEL-NOTE", 10)  # else: file-level comment dropped
    tui.keys("}")
    tui.wait_for("BB-REPLY-TO-DELETED", 8)  # else: reply to a deleted root dropped
    tui.check(r"\[deleted\]", "no [deleted] placeholder root")
    tui.quit()
