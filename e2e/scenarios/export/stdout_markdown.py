#!/usr/bin/env python3
# --stdout: :clip then quit prints the review markdown to stdout (captured in a file); stderr markers.
from e2e import TUICR_BIN, Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    out, err = s.dir / "stdout.txt", s.dir / "stderr.txt"
    tui = s.start_cmd(
        [
            "sh",
            "-c",
            f"exec '{TUICR_BIN}' --no-update-check --stdout -r HEAD~2..HEAD >'{out}' 2>'{err}'",
        ],
        cwd=repo,
    )
    tui.wait_for("Files · 0/8").keys("Escape")
    tui.cursor_on("4 ▌ second line")
    tui.comment("export me please")
    tui.cmd("clip")
    tui.wait_exit()  # with --stdout, :clip prints the export and ends the session
    # (the slug lists the range newest..oldest as the selector shows it; the commits line reads oldest first)
    # the whole document, not scattered substrings: session heading, intro, the reviewed scope (newest commit first),
    # the "Local tuicr Comments" section with ONE numbered item anchored at file:line, and the machine-readable stderr
    # markers
    new, old = (
        repos.git(repo, "rev-parse", "--short=7", "HEAD"),
        repos.git(repo, "rev-parse", "--short=7", "HEAD~1"),
    )  # HEAD~2..HEAD = these two commits
    want = (
        f"## Session: repo@main/commits/{new}..{old}\n\nI reviewed your code and have the following comments. "
        f"Please address them.\n\nReviewing commits: {old}, {new}\n\n## Local tuicr Comments\n\n"
        "1. `README.md:4` - export me please"
    )
    got = out.read_bytes().decode().rstrip("\n")
    assert got == want, f"exported markdown differs\n--- want\n{want}\n--- got\n{got}"
    err_text = err.read_bytes().decode()
    errlines = err_text.split("\n")
    assert f"tuicr-session: repo@main/commits/{new}..{old}" in errlines, f"stderr lacks tuicr-session\n{err_text}"
    assert "tuicr-summary: reviewed 0/8 files, 1 comment added" in errlines, f"stderr lacks the summary\n{err_text}"
