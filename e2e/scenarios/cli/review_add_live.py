#!/usr/bin/env python3
# `tuicr review add` from a second process while the TUI is open: comment appears live; list shows active.
import json

from e2e import Scenario, grep, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    repo = s.dir / "repo"
    active = [x["slug"] for x in json.loads(s.cli("review", "list", "--repo", repo)) if x.get("active")]
    assert active, f"no active session in review list: {s.cli('review', 'list', '--all')}"
    slug = active[0]
    print(f"slug={slug}")

    def sess(cmd: str, *args: object) -> str:
        return s.cli("review", cmd, "--repo", repo, "--session", slug, *args)

    sess(
        "add",
        "--target-file",
        "README.md",
        "--line",
        "4",
        "--side",
        "new",
        "--type",
        "issue",
        "cli injected note",
    )
    tui.wait_for("cli injected note", 15)  # picked up by the review watcher (default 1s poll)
    sess("add", "cli review-level note")
    tui.wait_for("cli review-level note", 15)  # merged live too: it shifts the rows, so wait before moving the cursor
    assert grep("cli injected note", sess("comments")), "comments lacks injected"
    # TUI-side comment coexists and is not overwritten
    tui.cursor_on("3   first line")
    tui.comment("tui side note")
    tui.save()
    out = sess("comments")
    for t in ("cli injected note", "cli review-level note", "tui side note"):
        assert grep(t, out), f"lost: {t}\n{out}"
    tui.quit()
