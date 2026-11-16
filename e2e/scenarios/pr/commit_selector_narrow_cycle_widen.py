#!/usr/bin/env python3
# PR inline commit selector: Enter narrows to one commit (range diff via compare), ( ) cycle, Space widens.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open()
    tui.expect("▣   2222222 +third commit").expect("second commit").expect("first commit")
    tui.keys(";k")
    tui.keys("j", "Enter")  # cursor on bbbbbbb (second commit) -> only it
    tui.wait_for(r"Commit Message \(bbbbbbb\)").wait_for("line from commit B")
    tui.forge_expect(r"compare/[0-9a-f]+\.\.\.b{40}")
    tui.reject("hello from the PR")
    # ) steps to the next older commit (list is newest first), then back to the whole PR
    tui.keys(")").wait_for(r"Commit Message \(aaaaaaa\)").wait_for("line from commit A")
    tui.forge_expect(r"compare/[0-9a-f]+\.\.\.a{40}")
    tui.keys(")").wait_gone(r"Commit Message \(").wait_for("new line").wait_for("hello from the PR")
    # ( goes the other way: from the whole PR to the newest commit
    tui.keys("(").wait_for(r"Commit Message \(2222222\)")
    tui.forge_expect(r"compare/[0-9a-f]+\.\.\.2{40}")
    tui.wait_for(r"A greet\.rs").wait_gone(r"▢ M README\.md")
    tui.quit()
