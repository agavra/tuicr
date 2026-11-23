#!/usr/bin/env python3
# Covers #901: transparent_background (default on) must survive runtime theme changes. Baseline painted the opaque
# palette bg (light = 245;243;232) after `:theme light` and during picker previews.
from e2e import Scenario, Tui, because, repos, scenario, wait_cmd

OPAQUE_LIGHT_BG = "48;2;245;243;232"
OUTSIDE = (1, 7)  # rows above the picker popup: header, file tree, diff


def cnt(tui: Tui) -> int:
    return tui.screen(colors=True, join=False).count(OPAQUE_LIGHT_BG)


def changed_from(tui: Tui, before: int, what: str) -> int:
    """Wait until the colours outside the picker differ from `before`; the new hash."""
    with because(what):
        wait_cmd(10, lambda: tui.colors_hash(OUTSIDE) != before)
    return tui.settle().colors_hash(OUTSIDE)


@scenario
def main(s: Scenario) -> None:
    # control: with transparent_background off, light paints this exact background (so counting it means something)
    opaque = s.name + "_opaque"
    repos.write(s.path_of(opaque) / "config" / "tuicr" / "config.toml", "transparent_background = false\n")
    ctl = s.start("--theme", "dark", "-r", "HEAD~2..HEAD", cwd=repos.git_repo(s.dir / "repo"), name=opaque, keep=True)
    ctl.wait_for("Files · 0/8").keys("Escape")
    ctl.cmd("theme light").wait_for(r"Theme: light \(this session").settle()
    assert cnt(ctl) > 0, f"precondition: opaque light theme paints no {OPAQUE_LIGHT_BG} background"
    ctl.quit()

    # start on dark explicitly: the default follows the terminal's appearance and may already be light
    tui = s.start("--theme", "dark", "-r", "HEAD~2..HEAD", cwd=s.dir / "repo")
    tui.wait_for("Files · 0/8").keys("Escape")
    dark = tui.settle().colors_hash(OUTSIDE)
    tui.cmd("theme light").wait_for(r"Theme: light \(this session")
    light = changed_from(tui, dark, ":theme light did not change the colours")
    c = cnt(tui)
    assert c == 0, f":theme light painted {c} opaque panel background cells despite transparent_background"
    # the picker opens on `dark` without previewing it: j previews light (already applied), k dark, j light again
    tui.cmd("theme").wait_for("Switch Theme")
    tui.keys("j", "k")
    previewed_dark = changed_from(tui, light, "picker preview of dark did not change the colours")
    tui.keys("j")
    changed_from(tui, previewed_dark, "picker preview of light did not change the colours")
    c = cnt(tui)
    assert c == 0, f"picker preview painted {c} opaque panel background cells"
    tui.keys("Escape").quit()
