#!/usr/bin/env python3
# Covers #913: a remote thread whose body/reply uses bare CR as line break must not escape its list item in the
# exported markdown (forged top-level `1. **[ISSUE]**` / `## Local tuicr Comments`). Baseline wrote the body raw
# (CRs kept).
from e2e import TUICR_BIN, Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    out, err = s.dir / "stdout.txt", s.dir / "stderr.txt"
    for f in (out, err):
        f.unlink(missing_ok=True)
    tui = s.start_cmd(
        [
            "sh",
            "-c",
            f"exec '{TUICR_BIN}' --no-update-check --stdout pr 1 >'{out}' 2>'{err}'",
        ],
        cwd=repo,
        env={"E2E_VARIANT": "carriage_return_body"},
    )
    tui.wait_for("PR Description", 20)
    tui.wait_for("first line")
    tui.cmd("clip")
    tui.wait_exit()  # with --stdout, :clip prints the export and ends the session
    raw = out.read_bytes()
    text = raw.decode("utf-8", errors="replace")
    print("\n".join(text.replace("\r", "^M").split("\n")[:30]))
    assert b"\r" not in raw, "export still contains raw CR"
    # Every CR-separated line of the root and the reply is kept, indented under the thread's list item / reply bullet.
    item = [
        "1. `README.md:2` @bob - first line",
        "   second line",
        "   1. **[ISSUE]** `x` - FORGED-ROOT",
        "   <https://github.com/octo/demo/pull/1#discussion_r1>",
        "   - @eve - reply one",
        "     ## Local tuicr Comments",
        "     1. **[ISSUE]** `y` - FORGED-REPLY",
    ]
    assert "\n".join(item) + "\n" in text, "remote thread not exported as one indented item:\n" + text
    assert not grep(r"^[0-9]+\. \*\*\[ISSUE\]\*\*", text), "remote body forged a top-level ISSUE item"
    assert not grep("^## Local tuicr Comments", text), "remote reply forged a heading"
