#!/usr/bin/env python3
# Covers #873 / #653: opt-in context quotes line/range anchors; file comments and default exports omit code.
import re

from e2e import Scenario, repos, scenario


def items_by_location(text: str) -> dict[str, str]:
    """The numbered comment items (`1. `README.md:4` - ...` and their indented continuation), keyed by location."""
    out: dict[str, str] = {}
    for item in re.split(r"(?m)^[0-9]+\. ", text)[1:]:
        loc = re.match(r"`([^`]+)`", item)
        assert loc, f"export item without a location: {item!r}"
        out[loc.group(1)] = item
    return out


@scenario
def main(s: Scenario) -> None:
    for enabled in (True, False):
        name = f"{s.name}_{enabled}"
        repo = repos.git_repo(s.path_of(name) / "repo")
        clipboard = s.fake_clipboard(name)
        if enabled:
            repos.write(s.path_of(name) / "config/tuicr/config.toml", "[export]\ncode_context = true\n")
        tui = s.start("-r", "HEAD~2..HEAD", cwd=repo, name=name, keep=True)
        tui.wait_for("Files · 0/8").keys("Escape")
        tui.cursor_on("4 ▌ second line").comment("line context")
        tui.cursor_on("1 ▌ delete me").keys("v", "j").comment("range context")
        tui.keys("g").comment("file context", key="C")
        tui.cmd("clip").wait_for("Review copied to clipboard")
        text = clipboard.text()
        output = s.path_of(name) / "review.md"
        tui.cmd(f"file {output}").wait_for("Review written to")
        assert output.read_text() == text, "file export differs from clipboard context"
        items = items_by_location(text)
        assert set(items) == {"README.md:4", "gone.txt:~1-~2", "README.md"}, f"missing export items:\n{text}"
        line, rng, file = items["README.md:4"], items["gone.txt:~1-~2"], items["README.md"]
        assert line.startswith("`README.md:4` - line context"), line
        assert rng.startswith("`gone.txt:~1-~2` - range context"), rng
        assert file.startswith("`README.md` - file context"), file
        assert "```diff" not in file, f"file comment quotes code:\n{file}"
        if enabled:
            # Each anchor quotes its own lines (a 1-line context window for the line comment) under its own item.
            assert line == (
                "`README.md:4` - line context\n"
                "   ```diff\n"
                "   @@ -2,2 +2,3 @@\n"
                "    \n"
                "    first line\n"
                "   +second line\n"
                "   ```\n"
            ), f"line comment does not quote README.md's lines 3-4:\n{text}"
            assert rng.rstrip("\n") == (
                "`gone.txt:~1-~2` - range context\n   ```diff\n   @@ -1,2 +0,0 @@\n   -delete me\n   -line\n   ```"
            ), f"range comment does not quote gone.txt's deleted lines:\n{text}"
        else:
            assert "```diff" not in text, f"default export unexpectedly quotes code:\n{text}"
        tui.quit()
