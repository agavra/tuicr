#!/usr/bin/env python3
# Esc in the target selector (:commits / :sessions) opened from an active review returns to the diff.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.open().wait_for("Files · 0/8")
    tui.cmd("commits").wait_for(" SELECT ")
    tui.keys("Escape").wait_gone(" SELECT ").expect("Files · 0/8")
    tui.cmd("sessions").wait_for(" SELECT ")
    tui.keys("Escape").wait_gone(" SELECT ").expect("Files · 0/8")
    tui.quit()
