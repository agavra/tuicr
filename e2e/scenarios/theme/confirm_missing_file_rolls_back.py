#!/usr/bin/env python3
# Covers #901: preview a local theme, delete its file, then confirm. Confirm must fail, restore the original theme
# and show an error. Baseline announced "Theme: zz-local" and kept the previewed colours.
import re

from e2e import E2E_DIR, Scenario, because, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.open().wait_for("Files · 0/8").keys("Escape")
    themes = s.dir / "config" / "tuicr" / "themes"
    themes.mkdir(parents=True)
    teal = (E2E_DIR.parent / "examples" / "tuicr-teal.toml").read_text()
    local = themes / "zz-local.toml"
    local.write_text(re.sub(r"(?m)^syntax_theme.*", "", teal))
    base = tui.colors_hash(1, (3, 30))
    tui.cmd("theme").wait_for("Switch Theme")
    tui.keys("/").type("zz-local").keys("Enter").settle(0.4)
    assert tui.colors_hash(1, (3, 30)) != base, (
        f"local theme preview did not change colours\n{tui.screen()}"
    )
    local.unlink()
    tui.keys("Enter").wait_for("Unknown theme 'zz-local'").settle(0.4)
    with because("announced a theme that failed to apply"):
        tui.reject("Theme: zz-local", 0.3)
    assert tui.colors_hash(1, (3, 30)) == base, (
        "failed confirm did not restore the original theme"
    )
    tui.quit()
