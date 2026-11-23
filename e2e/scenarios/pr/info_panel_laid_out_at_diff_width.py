#!/usr/bin/env python3
# Covers #904: the PR description panel must be laid out at the real diff width on the first frame. Baseline wrapped
# the body at a stale (0 / previous) width, so a one-row description showed split over several rows until a later
# redraw. Each check judges the FIRST screen that shows the body (polled every 20 ms, no settle in between), so a stale
# frame fixed by a later redraw still fails.
import time

from e2e import Failure, Scenario, Tui, grep, repos, scenario

BODY = "Ship-it-line: "
WHOLE = "Ship-it-line: .*wide OK"  # the whole one-row body on one screen line


def first_screen_with_body(tui: Tui, before: str | None = None, timeout: float = 10) -> str:
    """The first captured screen that differs from `before` and shows the body's first words."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        scr = tui.screen()
        if scr != before and grep(BODY, scr):
            return scr
        time.sleep(0.02)
    raise Failure(f"no screen showing /{BODY}/ within {timeout:g}s", tui.screen())


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo", origin="https://github.com/octo/demo.git")
    tui = s.start("pr", "1", cwd=repo, env={"E2E_VARIANT": "long_pr_body"})  # no wait: the first frame is the point
    scr = first_screen_with_body(tui, timeout=20)
    assert grep(WHOLE, scr), f"first frame wrapped the description at a stale width:\n{scr}"
    # a narrow terminal wraps the body (90 columns are too few for it)...
    tui.resize(90, 45).settle()
    narrow = tui.screen()
    assert not grep(WHOLE, narrow), f"the body did not wrap at 90 columns:\n{narrow}"
    # ...and widening again lays it out at the new width in the very frame drawn after the resize
    tui.resize(160, 45)
    scr = first_screen_with_body(tui, before=narrow)
    assert grep(WHOLE, scr), f"first frame after widening wrapped the description at the old width:\n{scr}"
    tui.quit()
