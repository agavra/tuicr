#!/usr/bin/env python3
# A changed binary file must be labelled as binary, not "(no changes)".
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.diff()
    tui.expect(r"═══ data\.bin \[M\]").expect(r"\(binary file\)")
    # data.bin's own section, from its header to the next file's: labelled binary, not "(no changes)"
    lines = tui.diff_pane(tui.screen()).split("\n")
    start = next(i for i, ln in enumerate(lines) if "═══ data.bin" in ln)
    end = next((i for i in range(start + 1, len(lines)) if "═══ " in lines[i]), len(lines))
    section = "\n".join(lines[start:end])
    assert "(binary file)" in section, f"data.bin is not labelled binary:\n{section}"
    assert "(no changes)" not in section, f"data.bin is labelled (no changes):\n{section}"
    tui.quit()
