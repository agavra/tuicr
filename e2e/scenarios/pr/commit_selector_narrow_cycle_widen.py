#!/usr/bin/env python3
# PR inline commit selector: Enter narrows to one commit (range diff via compare), ( ) cycle, Space widens.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open()
    tui.expect("▣   2222222 +third commit").expect("second commit").expect("first commit")
    tui.keys(";k")
    mark = tui.forge_mark()
    tui.keys("j", "Enter")  # cursor on bbbbbbb (second commit) -> only it
    tui.wait_for(r"Commit Message \(bbbbbbb\)").wait_for("line from commit B")
    tui.forge_expect(r"compare/a{40}\.\.\.b{40}", since=mark)  # else: narrowing did not fetch the commit's range
    tui.reject("hello from the PR")
    # ) steps to the next older commit (list is newest first), then back to the whole PR
    mark = tui.forge_mark()
    tui.keys(")").wait_for(r"Commit Message \(aaaaaaa\)").wait_for("line from commit A")
    tui.forge_expect(r"compare/[0-9a-f]+\.\.\.a{40}", since=mark)
    tui.keys(")").wait_gone(r"Commit Message \(").wait_for("new line").wait_for("hello from the PR")
    # ( goes the other way: from the whole PR to the newest commit
    mark = tui.forge_mark()  # the open already fetched compare/<base>...2222…: only a call made by `(` counts
    tui.keys("(").wait_for(r"Commit Message \(2222222\)")
    tui.forge_expect(r"compare/b{40}\.\.\.2{40}", since=mark)
    tui.wait_for(r"A greet\.rs").wait_gone(r"▢ M README\.md")
    # Space widens: adding bbbbbbb to the selected 2222222 shows the two-commit range bbbbbbb^..2222222
    tui.keys(";k").keys("j").wait_for("▸.*bbbbbbb")
    mark = tui.forge_mark()
    tui.keys("Space").wait_gone(r"Commit Message \(")
    tui.wait_for("▣.*bbbbbbb").wait_for("▢.*aaaaaaa")
    tui.forge_expect(r"compare/a{40}\.\.\.2{40}", since=mark)  # else: Space did not widen to the two-commit range
    # ... and adding aaaaaaa too selects every commit: the whole PR again
    tui.keys("j").wait_for("▸.*aaaaaaa").keys("Space")
    tui.wait_for("▣.*aaaaaaa").wait_for("hello from the PR").wait_for("new line")
    tui.reject(r"Commit Message \(|▢.*(2222222|bbbbbbb|aaaaaaa)")
    tui.quit()
