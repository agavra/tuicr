#!/usr/bin/env python3
# Gitea drafts read-back: the PENDING review created by `:submit draft` is read back (its summary body),
# so the post-submit prune does not delete the local review-level comment. Before: the PENDING summary was filtered out.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.forge_open("https://gitea.example.com/octo/demo.git")
    tui.keys(";c").wait_for("ctrl-s save").keys("gt overall draft").settle().keys("Enter")
    tui.wait_for("Review comment added")
    tui.cmd("submit draft").wait_for("Push pending|Submit review to").keys("y")
    tui.wait_for("Pushed pending Gitea review", 15)
    tui.forge_expect(r'pulls/1/reviews"\], "stdin": "\{.*gt overall draft')  # else: no review POST
    # the PENDING review is read back and listed in the PR description panel (not pruned/hidden)
    tui.wait_for("gitea @me pending")  # else: pending review summary not read back
    tui.expect("gt overall draft")
    tui.quit()
