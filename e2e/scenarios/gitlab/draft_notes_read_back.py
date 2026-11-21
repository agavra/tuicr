#!/usr/bin/env python3
# after `:submit draft` on GitLab the pushed drafts (inline + review body) are read back from
# /draft_notes. Before the fix the refetch found none and prune_locked_comments deleted the only local copy.
# The fake stores and serves back GitLab's own draft-note shape (id, author_id, note, position), not the request.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.forge_open("https://gitlab.com/octo/demo.git")
    tui.keys("}").cursor_on("1[╭ ]▌ pub fn greet")
    tui.keys("c").wait_for("COMMENT").keys("gl draft inline").keys("Enter").wait_for("gl draft inline")
    tui.keys(";c").wait_for("COMMENT").keys("gl draft body").keys("Enter").wait_for("gl draft body")
    tui.cmd("submit draft").wait_for("Inline: 1").keys("y")
    tui.wait_for("Pushed pending GitLab review", 15)
    # both drafts really reached the forge, as draft notes (the inline one anchored, the body one without position)
    posts = tui.forge_posts("glab")
    assert [c["argv"][1] for c in posts] == ["projects/octo%2Fdemo/merge_requests/1/draft_notes"] * 2, posts
    assert sorted(c["payload"]["note"] for c in posts) == ["gl draft body", "gl draft inline"], posts
    # ...and the refetch shows them back as remote draft threads, not pruned
    tui.wait_for("R src/greet.rs:1")  # else: pushed inline draft was pruned: not read back from /draft_notes
    tui.expect("R ● review")  # else: the pushed review body was not read back from /draft_notes
    tui.expect("gl draft inline").expect("gl draft body")
    tui.quit()
