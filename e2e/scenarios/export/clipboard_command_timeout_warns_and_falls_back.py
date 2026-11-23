#!/usr/bin/env python3
# Covers #895: a hanging clipboard_command is killed after 5 s; the export lands via the normal order with a warning.
import json
import os
import sys
import time

from e2e import Scenario, repos, scenario, wait_file

# The fallback copies through pbcopy on macOS and xclip elsewhere; both are recording fakes.
VIA = "pbcopy" if sys.platform == "darwin" else "xclip"


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    clipboard = s.fake_clipboard()
    pid_file = s.dir / "command.pid"
    command = f"echo $$ > '{pid_file}'; exec sleep 60"
    repos.write(s.dir / "config/tuicr/config.toml", f"clipboard_command = {json.dumps(command)}\n")
    tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, keep=True)
    tui.wait_for("Files · 0/8").keys("Escape")
    tui.cursor_on("4 ▌ second line").comment("hanging clipboard command")
    started = time.monotonic()
    tui.cmd("clip")
    pid = int(wait_file(pid_file).read_text().strip())
    tui.wait_for(rf"clipboard_command timed out after 5s; copied via {VIA}", timeout=20)
    elapsed = time.monotonic() - started
    assert 4 <= elapsed < 15, f"timeout took {elapsed:.1f}s, expected about 5s"
    actual = clipboard.text()
    assert "`README.md:4` - hanging clipboard command" in actual, actual
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        pass
    else:
        raise AssertionError(f"timed-out command {pid} is still running or was not reaped")
    tui.reject("Review copied to clipboard")
    tui.quit()
