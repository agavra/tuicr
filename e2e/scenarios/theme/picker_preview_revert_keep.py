#!/usr/bin/env python3
# :theme opens the picker; j previews live (colours change), Esc reverts, Enter keeps; :theme <name> applies directly
# (the same colours as starting with --theme <name>); an unknown name is an error.
from e2e import Scenario, Tui, because, scenario, wait_cmd

# Rows above the picker popup (centred in the diff pane) and clear of the status line: header, file tree, diff.
OUTSIDE = (1, 7)


def colours(tui: Tui) -> int:
    return tui.colors_hash(OUTSIDE)


def wait_colours(tui: Tui, ok: object, what: str) -> int:
    """Poll until ok(hash of OUTSIDE) holds; the hash."""
    pred = ok if callable(ok) else (lambda h: h == ok)
    with because(what):
        wait_cmd(10, lambda: pred(colours(tui)))
    return colours(tui)


@scenario
def main(s: Scenario) -> None:
    # an explicit start theme: the default follows the terminal's appearance (light on some hosts)
    tui = s.open("--theme", "dark", "-r", "HEAD~2..HEAD").wait_for("Files · 0/8").keys("Escape").settle()
    base = colours(tui)
    tui.cmd("theme")
    tui.wait_for("Switch Theme").expect("gruvbox-dark").expect("nord-dark")
    assert colours(tui) == base, "opening the picker changed the colours outside it before any preview"
    tui.keys("j")  # dark -> light
    light = wait_colours(tui, lambda h: h != base, "live preview did not change colours")
    tui.keys("Escape").wait_gone("Switch Theme")
    wait_colours(tui, base, "Esc did not revert the preview")
    # Enter keeps the previewed theme
    tui.cmd("theme").wait_for("Switch Theme").keys("j")
    wait_colours(tui, light, "the preview of `light` changed")
    tui.keys("Enter").wait_gone("Switch Theme").wait_for(r"Theme: light \(this session")
    tui.settle()
    assert colours(tui) == light, "Enter did not keep the previewed theme"
    # direct name: the same colours as a TUI started with --theme nord-dark
    ref = s.start("--theme", "nord-dark", "-r", "HEAD~2..HEAD", cwd=s.dir / "repo", name=s.name + "_ref")
    ref.wait_for("Files · 0/8").keys("Escape").settle()
    nord = colours(ref)
    assert nord not in (base, light), "precondition: nord-dark colours the rows like dark or light"
    ref.quit()
    tui.cmd("theme nord-dark").wait_for(r"Theme: nord-dark \(this session")
    wait_colours(tui, nord, ":theme nord-dark did not apply nord-dark")
    # unknown name
    tui.cmd("theme no-such-theme-xyz")
    tui.wait_for("Unknown theme 'no-such-theme-xyz'")
    tui.settle()
    assert colours(tui) == nord, "an unknown :theme name changed the colours"
    tui.quit()
