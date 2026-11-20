#!/usr/bin/env python3
# Covers #895 / #718: a custom command gets the complete export first; failure warns (on stderr after a :wq copy),
# unset uses the normal order.
import json
import shlex
import sys

from e2e import TUICR_BIN, Scenario, repos, scenario, wait_file

# The fallback copies through pbcopy on macOS and xclip elsewhere; both are recording fakes.
VIA = "pbcopy" if sys.platform == "darwin" else "xclip"


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    expected = None
    status = {
        "unset": "Review copied to clipboard",
        "success": "Review copied to clipboard",
        "failure": rf"clipboard_command failed \(exit status: 1\); copied via {VIA}",
        "short_read": f"clipboard_command did not read all copied text; copied via {VIA}",
    }
    for mode in ("unset", "success", "failure", "short_read"):
        name = f"{s.name}_{mode}"
        d = s.path_of(name)
        clipboard = s.fake_clipboard(name)
        output = d / "command with spaces.txt"
        if mode != "unset":
            reader = "head -c 1" if mode == "short_read" else "cat"
            command = f"{reader} > {shlex.quote(str(output))}"
            if mode == "failure":
                command += "; exit 1"
            repos.write(d / "config/tuicr/config.toml", f"clipboard_command = {json.dumps(command)}\n")
        tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, name=name, keep=True)
        tui.wait_for("Files · 0/8").keys("Escape")
        tui.cursor_on("4 ▌ second line").comment("complete custom clipboard export")
        large = "early-reader payload " + "x" * 262_144
        if mode == "short_read":
            sessions = json.loads(s.cli("review", "list", "--repo", repo, name=name))
            slug = next(item["slug"] for item in sessions if item["active"])
            s.cli(
                "review",
                "add",
                "--repo",
                repo,
                "--session",
                slug,
                "--input",
                "-",
                input=json.dumps({"content": large}),
                name=name,
            )
            tui.cmd("e").wait_for("Reloaded")
        tui.cmd("clip").wait_for(status[mode])
        if mode == "unset":
            expected = clipboard.text()
            assert "`README.md:4` - complete custom clipboard export" in expected, expected
        elif mode == "short_read":
            assert wait_file(output).read_bytes() == b"#", "command did not stop after one byte"
            actual = clipboard.text()
            assert large in actual and "`README.md:4` - complete custom clipboard export" in actual, actual[:500]
        else:
            assert wait_file(output).read_text() == expected, "command stdin differs from the complete export"
            if mode == "success":
                assert not clipboard.copied, "a local tool ran after the custom command succeeded"
            else:
                assert clipboard.text() == expected, "a failed command did not fall back with the complete export"
        tui.quit()

    # The :wq copy offer quits at once, so its fallback warning reaches stderr after teardown.
    name = f"{s.name}_write_quit"
    d = s.path_of(name)
    clipboard = s.fake_clipboard(name)
    err = d / "stderr.txt"
    repos.write(d / "config/tuicr/config.toml", 'clipboard_command = "exit 1"\n')
    argv = ["sh", "-c", f"exec '{TUICR_BIN}' --no-update-check -r HEAD~2..HEAD 2>'{err}'"]
    tui = s.start_cmd(argv, cwd=repo, name=name, keep=True)
    tui.wait_for("Files · 0/8").keys("Escape")
    tui.cursor_on("4 ▌ second line").comment("complete custom clipboard export")
    tui.cmd("wq").wait_for(r"Copy review to clipboard\?")
    tui.keys("y").wait_exit()
    assert clipboard.text() == expected, "the :wq copy did not fall back with the complete export"
    warning = f"Warning: clipboard_command failed (exit status: 1); copied via {VIA}"
    assert warning in err.read_text().splitlines(), err.read_text()
