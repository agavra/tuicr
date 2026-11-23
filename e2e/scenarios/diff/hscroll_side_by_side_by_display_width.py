#!/usr/bin/env python3
# Covers #888: side-by-side: horizontal scroll (`l`, 4 columns) pans by display width and pads a wide glyph that
# the pan cuts in half, so the divider stays in place. Row 5 is `a` + CJK: skipping 4 columns cuts the 2nd wide glyph.
# Baseline pans by characters: row 5 jumps to a different place (no pad cell).
import unicodedata

from e2e import Scenario, because, repos, scenario


def borders(line: str) -> list[int]:
    """Display columns of the `│` cells of a screen line (wide glyphs take 2 cells, combining marks none)."""
    out, col = [], 0
    for ch in line:
        if ch == "│":
            out.append(col)
        col += 0 if unicodedata.combining(ch) else 2 if unicodedata.east_asian_width(ch) in "WF" else 1
    return out


@scenario
def main(s: Scenario) -> None:
    repo = repos.wide_repo(s.dir / "repo")
    tui = s.start("-r", "HEAD~1..HEAD", cwd=repo)
    tui.wait_for("Files · 0/4").keys("Escape")
    tui.cmd("set nowrap").cmd("diff")
    tui.wait_for("5 ▌a日本語のテキスト")
    tui.keys("l")
    with because("wide glyph cut by the pan was not padded (panned by characters, not cells)"):
        tui.wait_for("5 ▌ 語のテキスト")
    # the pane divider and the right border of row 5 stay in the columns of an all-ASCII row (row 4)
    scr = tui.screen()
    row4 = next((ln for ln in scr.split("\n") if "4 ▌i three changed" in ln), None)
    row5 = next((ln for ln in scr.split("\n") if "5 ▌ 語のテキスト" in ln), None)
    assert row4 and row5, f"rows 4 and 5 of the hunk missing\n{scr}"
    assert borders(row5) == borders(row4), f"row 5 borders at {borders(row5)}, row 4 at {borders(row4)}\n{scr}"
    tui.quit()
