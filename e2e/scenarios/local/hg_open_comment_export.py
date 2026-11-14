#!/usr/bin/env python3
# Mercurial journey end to end: open the uncommitted changes, comment on an added line, export with --stdout. The
# markdown
# anchors the comment to the right file:line. The fixture changes two files of one status class (modified) on purpose:
# mixed status classes are covered by local/hg_hunks_under_own_files. Needs real Mercurial (SKIP without it, FAIL with
# E2E_REQUIRE_HG=1).
import re

from e2e import TUICR_BIN, Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    s.use_hg()
    d = s.dir
    repo = repos.fresh(d / "repo")  # only modified files: see the note in the header
    repos.write(repo / "a.txt", "a_old\n")
    repos.write(repo / "z.txt", "z_old\n")
    repos.run(repo, "hg", "init", "-q", ".")
    repos.run(repo, "hg", "add", "-q", ".")
    repos.run(repo, "hg", "commit", "-q", "-u", "e2e", "-m", "base")
    repos.write(repo / "a.txt", "a_new\n")
    repos.write(repo / "z.txt", "z_new\n")
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
    tui.wait_for("Unstaged changes").keys("Enter")
    tui.wait_for("Files · 0/2").keys("Escape").cursor_on("1 ▌ z_new")
    tui.keys("c").wait_for("COMMENT")
    tui.keys("hg exported note").keys("Enter").wait_for("hg exported note")
    tui.cmd("clip").wait_exit()
    text, errtext = out.read_bytes().decode(), err.read_bytes().decode()
    assert grep(r"^I reviewed your code", text), f"export lacks the intro\n{text}"
    assert any(re.fullmatch(r"1\. `z\.txt:1` - hg exported note", line) for line in text.split("\n")), (
        f"export lacks the anchored comment\n{text}"
    )
    assert "tuicr-summary: reviewed 0/2 files, 1 comment added" in errtext, f"stderr lacks the summary\n{errtext}"
