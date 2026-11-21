#!/usr/bin/env python3
# File-tree filters: i include, e exclude, / path search, I / E clear; filters narrow the diff too.
from e2e import Scenario, because, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.keys(";h").settle()  # focus the file tree
    tui.keys("i").keys("^src/").keys("Enter")
    tui.wait_for("Files · 0/2").wait_gone(r"README\.md")
    tui.expect(r"main\.rs").expect(r"lib\.rs").expect(r"pub fn a\(\) -> u32")
    tui.keys("e").keys("lib").keys("Enter")
    tui.wait_for("Files · 0/1").wait_gone(r"lib\.rs")
    with because("the exclude filter hid src/lib.rs from the tree but not from the diff"):
        tui.expect(r"println!").reject(r"pub fn a\(\) -> u32")  # src/main.rs stays, src/lib.rs's lines go
    tui.keys("I").wait_for("Files · 0/7").expect(r"README\.md").reject(r"▢ M lib\.rs")
    tui.keys("E").wait_for("Files · 0/8").expect(r"lib\.rs")
    # invalid regex keeps the prompt open
    tui.keys("i").type("(bad").keys("Enter").wait_for("Invalid regex")
    tui.check(r"└ i \(bad", "an invalid regex closed the include prompt")
    tui.keys("Escape").wait_gone(r"└ i \(bad")
    # path search moves the tree selection
    tui.keys("/").keys("tabs").keys("Enter")
    tui.keys("Enter").wait_for(r"┌ tabs\.txt ─")
    tui.quit()
