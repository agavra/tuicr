#!/usr/bin/env python3
# PR open through fake gh: description panel (status, reviewers, checks), r collapses/expands it.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.pr_open()
    tui.expect("PR #1 Add greeting and tweak README").expect("More description text here")
    tui.expect("Status: OPEN · REVIEW REQUIRED").expect("Requested: @bob").expect("Approved: @carol")
    tui.expect("✓ build").expect("✗ lint")
    tui.expect("PR Mode · octo/demo#1")
    tui.keys("r")
    tui.wait_for("═══ ✓ PR #1").wait_gone("More description text here")
    tui.keys("r")
    tui.wait_gone("═══ ✓ PR #1").wait_for("More description text here")
    # marking description reviewed persists (:w)
    tui.keys("r").wait_for("═══ ✓ PR #1")
    tui.save()
    assert s.session()["pr_description_reviewed"] is True, "pr_description_reviewed not saved"
    tui.quit()
