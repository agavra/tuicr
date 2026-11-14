#!/usr/bin/env python3
# `tuicr review` on a saved session without a TUI: JSON payloads (stdin and literal), flag comments, `delete` by id,
# `clearc` (keeps review marks) and `clear` (resets them, then removes the emptied session file).
import json
from typing import Any

from e2e import Scenario, grep, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    repo = s.dir / "repo"
    tui.keys("r").wait_for("Files · 1/8").save().quit()  # a session with one reviewed file
    sessions = json.loads(s.cli("review", "list", "--repo", repo))
    assert len(sessions) == 1, sessions
    slug = sessions[0]["slug"]

    def sess(cmd: str, *args: str, **kw: Any) -> str:
        return s.cli("review", cmd, "--repo", repo, "--session", slug, *args, **kw)

    def listed(pattern: str) -> bool:
        return grep(
            pattern, sess("comments")
        )  # a failing `comments` fails the scenario instead of reading as "not listed"

    sess(
        "add",
        "--input",
        "-",
        input='{"type": "issue", "content": "from stdin json", "file": "README.md", "line": 4, "side": "new"}\n',
    )
    sess(
        "add",
        "--input",
        '{"comment_type": "suggestion", "content": "range from literal json", "target": '
        '{"type": "line_range", "file": "README.md", "start_line": 3, "end_line": 4, "side": "new"}}',
    )
    comment_id = json.loads(sess("add", "--target-file", "src/lib.rs", "--username", "alice", "flag file note"))["id"]

    c = json.loads(sess("comments"))
    c = c if isinstance(c, list) else c.get("comments", c)
    by = {x["content"]: x for x in c}
    assert set(by) == {"from stdin json", "range from literal json", "flag file note"}, sorted(by)
    x = by["from stdin json"]
    assert (x["location"], x["comment_type"], x["side"]) == ("README.md:4", "issue", "new"), x
    x = by["range from literal json"]
    assert (x["start_line"], x["end_line"], x["comment_type"]) == (3, 4, "suggestion"), x
    x = by["flag file note"]
    assert x["path"] == "src/lib.rs" and "start_line" not in x or x["start_line"] is None, x
    assert by["flag file note"]["author"] == "alice", by["flag file note"]

    assert comment_id in sess("delete", "--comment-id", comment_id), "delete did not echo the id"
    assert not listed("flag file note"), "deleted comment still listed"
    assert s.cli_result("review", "delete", "--repo", repo, "--session", slug, "--comment-id", comment_id).rc != 0, (
        "deleting a missing id succeeded"
    )
    sess("clearc")
    assert not listed("from stdin json"), "clearc left comments"
    assert s.session_json(), "clearc removed a session that still has a reviewed file"
    sess("clear")
    assert not s.session_json(), f"clear left an empty session behind: {s.session_json()}"
