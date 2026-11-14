#!/usr/bin/env python3
# Covers #888: side-by-side: horizontal scroll (`l`, 4 columns) pans by display width and pads a wide glyph that
# the pan cuts in half, so the divider stays in place. Row 5 is `a` + CJK: skipping 4 columns cuts the 2nd wide glyph.
# Baseline pans by characters: row 5 jumps to a different place (no pad cell).
from e2e import Scenario, because, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.wide_repo(s.dir / "repo")
    tui = s.start("-r", "HEAD~1..HEAD", cwd=repo)
    tui.wait_for("Files · 0/4").keys("Escape")
    tui.cmd("set nowrap").cmd("diff")
    tui.wait_for("5 ▌a日本語のテキスト")
    tui.keys("l")
    with because(
        "wide glyph cut by the pan was not padded (panned by characters, not cells)"
    ):
        tui.wait_for("5 ▌ 語のテキスト")
    # every pane row keeps the divider: each diff row of the hunk still holds both panes' separators
    scr = tui.screen()
    assert grep(r"^│.*││ +[0-9]+ .*│ +[0-9]+ ", scr), f"divider rows missing\n{scr}"
    tui.quit()
