#!/usr/bin/env python3
# jj journey end to end: open the working-copy change, comment on a line, export with --stdout. The markdown carries
# the jj
# jj scope (the working-copy change) and the comment anchored at its file:line. Needs real jj (SKIP without it, FAIL
# with
# E2E_REQUIRE_JJ=1).
import re

from e2e import TUICR_BIN, Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    s.use_jj()
    d = s.dir
    repo = repos.jj_repo(d / "repo")
    out, err = d / "stdout.txt", d / "stderr.txt"
    out.unlink(missing_ok=True)
    err.unlink(missing_ok=True)
    tui = s.start_cmd(
        [
            "sh",
            "-c",
            f"exec '{TUICR_BIN}' --no-update-check --stdout >'{out}' 2>'{err}'",
        ],
        cwd=repo,
    )
    tui.wait_for("wip").keys("Enter")
    tui.wait_for("Files · 0/3").keys("Escape").cursor_on("3 ▌ a wip")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("jj exported note").keys("Enter").wait_for("jj exported note")
    tui.cmd("clip").wait_exit()
    text, errtext = out.read_bytes().decode(), err.read_bytes().decode()
    assert grep(r"^I reviewed your code", text), f"export lacks the intro\n{text}"
    assert any(
        re.fullmatch(r"1\. `a\.txt:3` \(commit [0-9a-f]+\) - jj exported note", line) for line in text.split("\n")
    ), f"export lacks the anchored comment\n{text}"
    assert "tuicr-summary: reviewed 0/3 files, 1 comment added" in errtext, f"stderr lacks the summary\n{errtext}"
