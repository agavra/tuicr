#!/usr/bin/env python3
# after `:submit draft` on GitLab the pushed drafts (inline + review body) are read back from
# /draft_notes. Before the fix the refetch found none and prune_locked_comments deleted the only local copy.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.forge_open("https://gitlab.com/octo/demo.git")
    tui.keys("}").cursor_on("1[╭ ]▌ pub fn greet")
    tui.keys("c").wait_for("COMMENT").keys("gl draft inline").keys("Enter").wait_for("gl draft inline")
    tui.cmd("submit draft").wait_for("Inline: 1").keys("y")
    tui.wait_for("Pushed|pushed|Submitted", 15)
    # the draft note really reached the forge
    tui.forge_expect('merge_requests/1/draft_notes", "--method", "POST"')  # else: no draft POST
    tui.forge_expect("gl draft inline")
    # ...and the refetch shows it back as a remote draft thread (navigator row "R src/greet.rs:1"), not pruned
    tui.wait_for("R src/greet.rs:1")  # else: pushed draft was pruned: not read back from /draft_notes
    tui.quit()
