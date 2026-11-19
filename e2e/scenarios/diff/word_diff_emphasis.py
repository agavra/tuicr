#!/usr/bin/env python3
# Covers #184: a modified line emphasizes only the changed word, in the unified and side-by-side views. `:set
# noworddiff` / `:set worddiff!` turn it off and on, and it survives a resize: since the renderers rebuild their
# annotations when the width changes, the frame drawn for the resize lost the emphasis before #860's fix.
import re

from e2e import Failure, Scenario, Tui, repos, scenario

SGR = re.compile(r"\x1b\[([0-9;]*)m")
OLD = "let total = 17;"
NEW = "let total = 14;"
# 40-47 and 100-107: the eight named background colours and their bright variants
NAMED_BG = {str(n) for n in [*range(40, 48), *range(100, 108)]}


def backgrounds(line: str) -> list[tuple[str, str]]:
    """Each printed character of a `capture-pane -e` row with the background SGR it is drawn on ("" = default)."""
    cells: list[tuple[str, str]] = []
    bg = ""
    pos = 0
    for m in SGR.finditer(line + "\x1b[m"):
        cells.extend((ch, bg) for ch in line[pos : m.start()])
        pos = m.end()
        params = m.group(1).split(";") if m.group(1) else ["0"]
        i = 0
        while i < len(params):
            p = params[i]
            if p in ("0", "49"):
                bg = ""
            elif p == "48" and i + 1 < len(params):
                width = 5 if params[i + 1] == "2" else 3
                bg = ";".join(params[i : i + width])
                i += width - 1
            elif p in ("38", "58") and i + 1 < len(params):
                i += 4 if params[i + 1] == "2" else 2
            elif p in NAMED_BG:
                bg = p
            i += 1
    return cells


def emphasis(tui: Tui, text: str, occurrence: int = 0) -> tuple[str, str, str]:
    """Backgrounds of `let` (unchanged), the number (changed) and `;` (unchanged) in an occurrence of `text`."""
    seen = 0
    screen = tui.screen(colors=True, join=False)
    for row in screen.split("\n"):
        cells = backgrounds(row)
        plain = "".join(ch for ch, _ in cells)
        start = plain.find(text)
        while start != -1:
            if seen == occurrence:
                at = text.index("1")
                return cells[start][1], cells[start + at][1], cells[start + len(text) - 1][1]
            seen += 1
            start = plain.find(text, start + 1)
    raise Failure(f"{text!r} (occurrence {occurrence}) is not on the screen", tui.screen())


def assert_emphasized(tui: Tui, text: str, where: str, occurrence: int = 0) -> None:
    word, number, tail = emphasis(tui, text, occurrence)
    assert word == tail, f"{where}: unchanged words of {text!r} differ in background ({word!r} vs {tail!r})"
    assert number != word, f"{where}: the changed number of {text!r} is not emphasized (all {word!r})"


def assert_plain(tui: Tui, text: str, where: str) -> None:
    word, number, tail = emphasis(tui, text)
    assert word == number == tail, f"{where}: {text!r} is not one background ({word!r}, {number!r}, {tail!r})"


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    body = 'fn main() {\n    LINE\n    println!("{}", total);\n}\n'
    repos.write(r / "totals.txt", body.replace("LINE", OLD))
    repos.git(r, "add", "totals.txt")
    repos.git(r, "commit", "-qm", "c1")
    repos.write(r / "totals.txt", body.replace("LINE", NEW))
    repos.git(r, "commit", "-qam", "c2")

    tui = s.start("-r", "HEAD~1..HEAD", cwd=r)
    tui.wait_for("Files · 0/2").wait_for(re.escape(NEW)).settle()
    assert_emphasized(tui, OLD, "unified, default on")
    assert_emphasized(tui, NEW, "unified, default on")

    tui.cmd("set noworddiff").wait_for("Word diff: off").settle()
    assert_plain(tui, OLD, "unified, :set noworddiff")
    assert_plain(tui, NEW, "unified, :set noworddiff")

    tui.cmd("set worddiff!").wait_for("Word diff: on").settle()
    assert_emphasized(tui, OLD, "unified, :set worddiff!")
    assert_emphasized(tui, NEW, "unified, :set worddiff!")

    tui.resize(120, 45).settle(0.4)
    assert_emphasized(tui, OLD, "unified, after a resize")
    assert_emphasized(tui, NEW, "unified, after a resize")

    tui.cmd("diff").wait_for(re.escape(OLD) + ".*│.*" + re.escape(NEW)).settle()
    assert_emphasized(tui, OLD, "side-by-side, old column")
    assert_emphasized(tui, NEW, "side-by-side, new column")
    tui.quit()
