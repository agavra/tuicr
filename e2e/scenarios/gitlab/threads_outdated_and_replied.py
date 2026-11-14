#!/usr/bin/env python3
# Covers #891: (a) a discussion whose position.head_sha differs from the MR head is outdated: hidden by the
# default `:comments unresolved`, shown muted with an "outdated" badge under `:comments all`. (b) a general comment that
# got a reply (positionless discussion with individual_note=false) stays visible instead of being dropped.
# Baseline: outdated thread shown as active by default, replied general comment dropped.
from e2e import Scenario, scenario

DISCUSSIONS = """\
[{"id":"stale","individual_note":false,"notes":[{"id":11,"body":"STALE-INLINE-NOTE","author":{"username":"carol"},"resolved":false,"system":false,
   "position":{"position_type":"text","head_sha":"1111111111111111111111111111111111111111","new_path":"src/greet.rs","new_line":1}}]},
 {"id":"fresh","individual_note":false,"notes":[{"id":12,"body":"FRESH-INLINE-NOTE","author":{"username":"dave"},"resolved":false,"system":false,
   "position":{"position_type":"text","head_sha":"2222222222222222222222222222222222222222","new_path":"src/greet.rs","new_line":2}}]},
 {"id":"gen","individual_note":false,"notes":[{"id":13,"body":"GENERAL-ROOT-NOTE","author":{"username":"erin"},"resolved":false,"system":false},
   {"id":14,"body":"GENERAL-REPLY-NOTE","author":{"username":"frank"},"resolved":false,"system":false}]}]
"""


@scenario
def main(s: Scenario) -> None:
    s.dir.mkdir(parents=True, exist_ok=True)
    disc = s.dir / "disc.json"
    disc.write_text(DISCUSSIONS)
    tui = s.forge_open("https://gitlab.com/octo/demo.git", env={"E2E_GL_DISCUSSIONS": str(disc)})
    # general comments render in the PR description pane (top); check before `}` scrolls them away
    tui.wait_for("GENERAL-ROOT-NOTE", 10)  # else: replied general comment dropped
    tui.keys("}")
    tui.wait_for("FRESH-INLINE-NOTE", 10)  # else: current-head thread missing
    tui.reject("STALE-INLINE-NOTE")  # else: outdated discussion shown as active by default
    tui.cmd("comments all")
    tui.wait_for("STALE-INLINE-NOTE", 5)  # else: outdated thread absent under :comments all
    tui.check(r"@carol outdated\]", "no outdated badge")
    tui.quit()
