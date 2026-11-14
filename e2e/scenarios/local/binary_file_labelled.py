#!/usr/bin/env python3
# A changed binary file must be labelled as binary, not "(no changes)".
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.expect(r"═══ data\.bin \[M\]")
    tui.expect(r"\(binary file\)")
    tui.quit()
