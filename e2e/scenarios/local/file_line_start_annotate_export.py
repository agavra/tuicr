#!/usr/bin/env python3
# `--file F --line N` opens the unchanged file with the cursor on line N (centered); a comment made there is exported
# with its `F:N` anchor by `:clip` under `--stdout`.
from e2e import TUICR_BIN, Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    d = s.dir
    repo = repos.long_file_repo(d / "repo")
    out = d / "stdout.txt"
    out.unlink(missing_ok=True)
    tui = s.start_cmd(
        [
            "sh",
            "-c",
            f"exec '{TUICR_BIN}' --no-update-check --stdout --file long.txt --line 70 >'{out}' 2>/dev/null",
        ],
        cwd=repo,
    )
    tui.wait_for("▶ +70 ▌ line 70")
    scr = tui.screen()
    assert grep(r"^ +49 ▌ line 49", scr) and grep(r"^ +9[01] ▌ line 9[01]", scr), (
        f"line 70 is not centered in the viewport\n{scr}"
    )
    tui.keys("c").wait_for("COMMENT")
    tui.keys("annotated at seventy").keys("Enter").wait_for("annotated at seventy")
    tui.cmd("clip").wait_exit()
    text = out.read_bytes().decode()
    assert grep(r"long\.txt:70.*annotated at seventy", text), f"export lacks the anchored comment:\n{text}"
