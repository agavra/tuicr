#!/usr/bin/env python3
# Configured `comment_types`: Tab / Shift-Tab cycle them (config order, then none) showing the config *label*; the
# export
# tags the comment with that label, leaves an untyped comment bare, and lists the type used, with its definition, in the
# legend.
from e2e import TUICR_BIN, Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    repo = repos.git_repo(d / "repo")
    repos.write(
        d / "config/tuicr/config.toml",
        """comment_types = [
  { id = "note", label = "question", definition = "ask for clarification" },
  { id = "issue", definition = "problems to fix" },
  { id = "nit", label = "nitpick", definition = "small tweaks" },
]
""",
    )
    out = d / "stdout.txt"
    out.unlink(missing_ok=True)
    tui = s.start_cmd(
        [
            "sh",
            "-c",
            f"exec '{TUICR_BIN}' --no-update-check --stdout -r HEAD~2..HEAD >'{out}' 2>/dev/null",
        ],
        cwd=repo,
        keep=True,
    )
    tui.wait_for("Files · 0/8").keys("Escape")
    tui.cursor_on("4 ▌ second line")
    tui.keys("c").wait_for(r"Add \[QUESTION\]")  # first configured type is the default
    tui.keys("Tab").wait_for(r"Add \[ISSUE\]")  # no label: the id
    tui.keys("Tab").wait_for(r"Add \[NITPICK\]")
    tui.keys("Tab").wait_for("Comment type: none")  # none closes the cycle
    tui.keys("Tab").wait_for(r"Add \[QUESTION\]")
    tui.keys("BTab").wait_for("Comment type: none")  # and Shift-Tab walks it backwards
    tui.keys("BTab").wait_for(r"Add \[NITPICK\]")
    tui.keys("tiny detail").keys("Enter").wait_for("tiny detail")
    tui.expect(r"\[NITPICK\]")  # the saved comment shows its label
    tui.keys("g").cursor_on("3   first line")
    tui.keys("c").wait_for(r"Add \[QUESTION\]")
    tui.keys("Tab", "Tab", "Tab").wait_for("Comment type: none")
    tui.keys("plain remark").keys("Enter").wait_for("plain remark")
    tui.cmd("clip").wait_exit()
    text = out.read_bytes().decode()
    assert grep(r"\*\*\[NITPICK\]\*\*.*README\.md:4.*tiny detail", text), f"export lacks the labelled comment:\n{text}"
    plain = [line for line in text.split("\n") if "plain remark" in line]
    assert not any("[" in line for line in plain), f"untyped comment got a tag:\n{plain}"
    assert plain, f"export lacks the untyped comment:\n{text}"
    assert "Comment types: NITPICK (small tweaks)" in text, f"legend lacks the used type and its definition:\n{text}"
