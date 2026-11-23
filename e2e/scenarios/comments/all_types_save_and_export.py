#!/usr/bin/env python3
# The four built-in classifications (note, suggestion, issue, praise) opted into through `comment_types`: one comment of
# each is saved through the TUI. Each is stored under its own id, exported with its own `[TYPE]` tag at its own
# file:line, and the legend names every type used with its definition.
from e2e import TUICR_BIN, Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    repo = repos.git_repo(d / "repo")
    repos.write(
        d / "config/tuicr/config.toml",
        """comment_types = [
  { id = "note", definition = "observation" },
  { id = "suggestion", definition = "improvement" },
  { id = "issue", definition = "must fix" },
  { id = "praise", definition = "well done" },
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

    def add(cursor: str, tabs: int, label: str, text: str) -> None:
        # <cursor regex> <tabs to press> <label shown> <text>
        tui.cursor_on(cursor)
        tui.keys("c").wait_for(r"Add \[NOTE\]")  # the first configured type is the default
        for _ in range(tabs):
            tui.keys("Tab")
        tui.wait_for(rf"Add \[{label}\]")
        tui.type(text).keys("Enter")
        tui.wait_saved_comment(text)

    add("4 ▌ second line", 0, "NOTE", "plain observation")
    add("2 ▌ 中文内容", 1, "SUGGESTION", "better approach")
    add(r"1 ▌ pub fn a\(\) -> u32 \{ 2 \}", 2, "ISSUE", "broken thing")
    add(r'2 ▌     println!\("hello"\);', 3, "PRAISE", "nicely done")
    tui.cmd("clip").wait_exit()
    text = out.read_bytes().decode()
    for want in (
        r"NOTE.*README\.md:4.*plain observation",
        r"SUGGESTION.*notes_cjk\.txt:2.*better approach",
        r"ISSUE.*src/lib\.rs:1.*broken thing",
        r"PRAISE.*src/main\.rs:2.*nicely done",
    ):
        assert grep(rf"^[0-9]+\. \*\*\[{want}", text), f"export lacks /{want}/:\n{text}"
    legend = "Comment types: NOTE (observation), SUGGESTION (improvement), ISSUE (must fix), PRAISE (well done)"
    assert legend in text, f"legend lacks the four types:\n{text}"
    # each comment stored under its own type id (the session was autosaved when each comment was added)
    stored = {
        c["content"]: c["comment_type"]
        for f in s.session()["files"].values()
        for cs in f["line_comments"].values()
        for c in cs
    }
    want = {
        "plain observation": "note",
        "better approach": "suggestion",
        "broken thing": "issue",
        "nicely done": "praise",
    }
    assert stored == want, f"stored comment types {stored}, want {want}"
