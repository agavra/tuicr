#!/usr/bin/env python3
# ? opens help; / searches it: the match is highlighted and scrolled into view, a miss says so; ? and q close it,
# :help opens it too.
import re

from e2e import Scenario, Tui, because, scenario, wait_cmd

TITLE = r"Help \(j/k scroll, h/l pan, / search\)"


def bg_of(tui: Tui, text: str) -> str | None:
    """The background (SGR parameters, e.g. '48;2;40;44;52'; None = default) under the first character of `text`."""
    line = next(
        (ln for ln in tui.screen(colors=True, join=False).split("\n") if text in re.sub(r"\x1b\[[0-9;]*m", "", ln)),
        None,
    )
    if line is None:
        return "<not on screen>"
    bg: str | None = None
    plain, bgs = "", []
    for m in re.finditer(r"\x1b\[([0-9;]*)m|([^\x1b])", line):
        if m.group(2) is not None:
            plain += m.group(2)
            bgs.append(bg)
            continue
        p = m.group(1).split(";")
        i = 0
        while i < len(p):
            c = p[i]
            if c in ("", "0", "49"):
                bg = None
            elif c in ("48",) and i + 1 < len(p):
                n = 5 if p[i + 1] == "2" else 3
                bg, i = ";".join(p[i : i + n]), i + n - 1
            elif c in ("38", "58") and i + 1 < len(p):
                i += 4 if p[i + 1] == "2" else 2
            elif re.fullmatch(r"4[0-7]|10[0-7]", c):
                bg = c
            i += 1
    return bgs[plain.index(text)]


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.keys("?").wait_for(TITLE).expect("Scroll down/up")
    # a match already in view gets the highlight background
    plain = bg_of(tui, "Search within diff")
    assert plain != "<not on screen>", f"precondition: 'Search within diff' not in the first help page\n{tui.screen()}"
    tui.keys("/").type("search within diff").keys("Enter")
    with because("help search did not highlight the matching line"):
        wait_cmd(10, lambda: bg_of(tui, "Search within diff") not in (plain, "<not on screen>"))
    hl = bg_of(tui, "Search within diff")
    # a match below the fold is scrolled into view and highlighted
    tui.reject("Cycle PR scope")
    tui.keys("/").type("cycle pr scope").keys("Enter")
    with because("help search did not scroll to a match below the fold"):
        tui.wait_for("Cycle PR scope")
    assert bg_of(tui, "Cycle PR scope") == hl, f"the scrolled-to match is not highlighted\n{tui.screen()}"
    # a miss says so and keeps the help open
    tui.keys("/").type("zzqqxx").keys("Enter").wait_for('No help matches for "zzqqxx"').check(TITLE, "help closed")
    tui.keys("?")
    tui.wait_gone(TITLE)
    tui.expect("Files · 0/8")
    tui.cmd("help").wait_for(TITLE)
    tui.keys("q").wait_gone(TITLE)
    tui.quit()
