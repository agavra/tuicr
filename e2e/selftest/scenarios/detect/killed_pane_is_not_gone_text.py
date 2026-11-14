#!/usr/bin/env python3
# Runner self-test (expect PASS): a pane that was killed mid-scenario must not satisfy "text is gone".
from e2e import NoPane, Scenario, must_fail, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.start_cmd(["sh", "-c", "echo visible-text; sleep 60"])
    tui.wait_for("visible-text", 5)
    tui.wait_gone("not-on-screen", 2)  # control: a live pane without the text passes
    tui.kill()
    must_fail(lambda: tui.wait_gone("visible-text", 1), "wait_gone passed on a killed pane", NoPane)
    must_fail(lambda: tui.reject("forbidden", 1), "reject passed on a killed pane", NoPane)
