#!/usr/bin/env python3
# option conflicts are checked on the merged root/tui/pr flags, and `pr` rejects local-review flags. Each invocation
# must exit with a conflict error instead of opening the TUI.
from e2e import Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    for args in [
        "-A tui --file README.md",
        "--file README.md tui --path src/",
        "-r HEAD tui -A",
        "-w tui --file README.md",
        "pr 1 --file README.md",
        "pr 1 -A",
        "-w pr 1",
    ]:
        r = s.cli_result("--no-update-check", *args.split(), cwd=repo)
        assert r.rc != 0, f"'tuicr {args}' exited 0 instead of rejecting the flags. Output: {r.out}"
        assert grep("cannot be|conflict", r.out), f"'tuicr {args}' not rejected with a conflict error. Output: {r.out}"
        print(f"ok: tuicr {args} rejected")
    # still allowed: -r with -w (split across levels)
    tui = s.start("-r", "HEAD~2..HEAD", "tui", "-w", cwd=repo)
    tui.wait_for("Files ·").quit()
