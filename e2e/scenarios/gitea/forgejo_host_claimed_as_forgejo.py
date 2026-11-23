#!/usr/bin/env python3
# Forgejo host detection (issue #518): a PR on codeberg.org (which runs Forgejo) is served by the
# fake `tea` and classified as Forgejo — the pending review's badge reads `[forgejo @me pending]`.
# Before the change, codeberg.org matched no Gitea signal (host contains neither "gitea" nor
# "forgejo" and the fake `tea logins list` is empty), so the remote fell through to the GitHub
# parser and the PR never loaded through tea at all.
from e2e import Scenario, scenario


@scenario
def main(s: Scenario) -> None:
    tui = s.forge_open("https://codeberg.org/octo/demo.git")
    # the fake tea (not gh) served the PR: without Forgejo host detection this call never happens
    tui.forge_expect(r'/repos/octo/demo/pulls/1"')
    tui.keys(";c").wait_for("ctrl-s save").keys("fj forgejo draft").settle().keys("Enter")
    tui.wait_for("Review comment added")
    tui.cmd("submit draft").wait_for("Push pending|Submit review to").keys("y")
    tui.wait_for("Pushed pending", 15)
    # the badge names the forge kind: Forgejo, not gitea
    tui.wait_for("[forgejo @me pending")
    tui.quit()
