#!/usr/bin/env python3
# File-tree filters: i include, e exclude, / path search, I / E clear; filters narrow the diff too.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.keys(";h").settle()  # focus the file tree
    tui.keys("i").keys("^src/").keys("Enter")
    tui.wait_for("Files · 0/2").wait_gone(r"README\.md")
    tui.expect(r"main\.rs").expect(r"lib\.rs")
    tui.keys("e").keys("lib").keys("Enter")
    tui.wait_for("Files · 0/1").wait_gone(r"lib\.rs")
    tui.keys("I").wait_for("Files · 0/7").expect(r"README\.md").reject(r"▢ M lib\.rs")
    tui.keys("E").wait_for("Files · 0/8").expect(r"lib\.rs")
    # invalid regex keeps the prompt open
    tui.keys("i").keys("(bad").keys("Enter").wait_for("[Ii]nvalid|[Rr]egex|error")
    tui.keys("Escape")
    # path search moves the tree selection
    tui.keys("/").keys("tabs").keys("Enter")
    tui.keys("Enter").wait_for(r"┌ tabs\.txt ─")
    tui.quit()
