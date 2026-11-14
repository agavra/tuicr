#!/usr/bin/env python3
# Covers #904: the PR description panel must be laid out at the real diff width on the first frame. Baseline wrapped
# the body at a stale (0 / previous) width, so a one-row description showed split over several rows until a later
# redraw.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open("long_pr_body")
    tui.settle()
    # narrow terminal wraps the body; widening again must relayout at the new width in the very frame drawn after the
    # resize
    tui.resize(90, 45).settle(0.4)
    print("\n".join(line[:100] for line in tui.screen().split("\n")[2:7]))
    tui.resize(160, 45).settle(0.4)
    print("\n".join(line[31:150] for line in tui.screen().split("\n")[0:12]))
    # description body is not on one row at the real width
    tui.expect("Ship-it-line: .*wide OK", 2)
    tui.quit()
