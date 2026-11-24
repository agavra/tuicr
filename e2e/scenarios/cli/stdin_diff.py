#!/usr/bin/env python3
import os
import subprocess
from pathlib import Path

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    target = Path(repo) / "README.md"
    original = target.read_text(encoding="utf-8")
    target.write_text(original + "\n// piped via --stdin\n", encoding="utf-8")
    diff = subprocess.check_output(["git", "diff"], cwd=repo, text=True)
    target.write_text(original, encoding="utf-8")

    tuicr_bin = os.environ.get(
        "TUICR_BIN",
        str(Path(__file__).resolve().parents[3] / "target/release/tuicr"),
    )
    tui = s.start_cmd(
        ["sh", "-c", 'printf \'%s\' "$TUICR_STDIN_DIFF" | "$1" --no-update-check --stdin', "_", tuicr_bin],
        cwd=repo,
        env={"TUICR_STDIN_DIFF": diff},
    )
    tui.wait_for(r"piped diff").wait_for(r"Files · 0/1").wait_for(r"README\.md").wait_for(r"7 ▌ // piped via --stdin")
    tui.quit()
