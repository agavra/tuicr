#!/usr/bin/env python3
# Covers #907: a user grammar syntect cannot compile (here `push: [[]]`) is skipped with a warning; tuicr still starts
# and the valid user grammar next to it still highlights. Baseline panics inside syntect at startup.
import re

from e2e import Scenario, because, repos, scenario, wait_cmd

SYNTAXES = ("config", "tuicr", "syntaxes")


def fg_colours(screen: str, text: str) -> set[str]:
    """Foreground colours (38;2;r;g;b) used on the screen line holding `text`."""
    line = next((ln for ln in screen.split("\n") if text in re.sub(r"\x1b\[[0-9;]*m", "", ln)), "")
    return set(re.findall(r"38;2;[0-9]+;[0-9]+;[0-9]+", line))


@scenario
def main(s: Scenario) -> None:
    r = repos.empty_repo(s.dir / "repo")
    bad = (
        "%YAML 1.2\n---\nname: PushEmpty\nscope: source.pushempty\nfile_extensions: [pushempty]\n"
        "contexts:\n  main:\n    - match: a\n      push: [[]]\n"
    )
    good = (
        "%YAML 1.2\n---\nname: Zork\nscope: source.zork\nfile_extensions: [zork]\n"
        "contexts:\n  main:\n    - match: '\\bkwzork\\b'\n      scope: keyword.control.zork\n"
    )
    repos.write(s.dir.joinpath(*SYNTAXES) / "PushEmpty.sublime-syntax", bad)
    repos.write(s.dir.joinpath(*SYNTAXES) / "Zork.sublime-syntax", good)
    repos.write(r / "b.pushempty", "aaa\n")
    repos.write(r / "c.zork", "plainword\nkwzork\n")
    tui = s.start("--file", "c.zork", cwd=r, keep=True)
    with because("tuicr did not start with a malformed user grammar present"):
        tui.wait_for(r"c\.zork")
    # the startup warning names the file, but its path is too long for the status line: PushEmpty is the only
    # grammar that can fail here (a failing Zork grammar fails the highlight check below)
    with because("the malformed grammar was not reported"):
        tui.wait_for("Failed to load syntax ")
    # the keyword line carries a token colour the plain line lacks (colouring can land a frame after the text)
    tui.wait_for("kwzork")

    def keyword_coloured() -> bool:
        scr = tui.screen(colors=True)
        return bool(fg_colours(scr, "kwzork") - fg_colours(scr, "plainword"))

    with because("the valid user grammar next to the broken one does not highlight: kwzork is coloured as plain text"):
        wait_cmd(10, keyword_coloured)
    tui.quit()
