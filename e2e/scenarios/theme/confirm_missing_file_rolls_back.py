#!/usr/bin/env python3
# Covers #901: preview a local theme, delete its file, then confirm. Confirm must fail, restore the original theme
# and show an error. Baseline announced "Theme: zz-local" and kept the previewed colours.
import re

from e2e import E2E_DIR, Scenario, because, scenario, wait_cmd


@scenario
def main(s: Scenario) -> None:
    tui = s.open().wait_for("Files · 0/8").keys("Escape")
    themes = s.dir / "config" / "tuicr" / "themes"
    themes.mkdir(parents=True)
    teal = (E2E_DIR.parent / "examples" / "tuicr-teal.toml").read_text()
    local = themes / "zz-local.toml"
    local.write_text(re.sub(r"(?m)^syntax_theme.*", "", teal))
    # rows above the picker popup (centred in the diff pane): header, file tree, diff
    outside = (1, 7)
    base = tui.colors_hash(outside)
    tui.cmd("theme").wait_for("Switch Theme")
    tui.keys("/").type("zz-local").keys("Enter")
    with because("local theme preview did not change the colours outside the picker"):
        wait_cmd(10, lambda: tui.colors_hash(outside) != base)
    local.unlink()
    tui.keys("Enter").wait_for("Unknown theme 'zz-local'").wait_gone("Switch Theme").settle(0.4)
    with because("announced a theme that failed to apply"):
        tui.reject("Theme: zz-local", 0.3)
    assert tui.colors_hash(outside) == base, "failed confirm did not restore the original theme"
    tui.quit()
