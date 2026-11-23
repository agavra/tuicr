#!/usr/bin/env python3
# Covers #952: `hunk_jump_align = "top"` puts the header of an off-screen hunk at the top of the diff, leaving
# `scroll_offset` rows above it, instead of the bottom edge that plain visibility leaves.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    base = "".join(f"line {i}\n" for i in range(1, 401))
    repos.write(r / "long.txt", base)
    repos.git(r, "add", "-A")
    repos.git(r, "commit", "-qm", "base")
    changed = "".join(f"line {i} changed\n" if 50 <= i <= 150 or i == 350 else f"line {i}\n" for i in range(1, 401))
    repos.write(r / "long.txt", changed)
    repos.write(
        s.dir / "config" / "tuicr" / "config.toml",
        'hunk_jump_align = "top"\nscroll_offset = 3\n',
    )
    tui = s.start("-w", cwd=r, keep=True)
    tui.wait_for("Files · 0/1").keys("Escape").settle()
    tui.keys("]").wait_for(r"▶.*@@ -47,")  # first hunk header
    tui.keys("]").wait_for(r"▶.*@@ -347,")  # second hunk header, below the viewport before the jump
    scr = tui.screen()
    header = next(i for i, line in enumerate(scr.split("\n")) if "▶" in line and "@@" in line)
    # Row 0 is the title bar and row 1 the diff pane's top border, so the first
    # content row is 2; `scroll_offset = 3` keeps three rows above the header.
    assert header == 5, f"hunk header at screen row {header}, want 5 (top + scroll_offset)\n{scr}"
    tui.quit()
