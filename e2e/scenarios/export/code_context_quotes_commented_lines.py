#!/usr/bin/env python3
# Covers #873 / #653: opt-in context quotes line/range anchors; file comments and default exports omit code.
from e2e import Scenario, grep, repos, scenario


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
        assert "`README.md:4` - line context" in text, text
        assert "`gone.txt:~1-~2` - range context" in text, text
        if enabled:
            assert text.count("```diff") == 2, f"expected context only for line and range comments:\n{text}"
            assert grep(r"^   @@ -1,2 \+0,0 @@$", text), text
            assert "   -delete me\n   -line\n" in text, text
            assert "   +second line\n" in text, text
            assert grep(r"^   @@ -[0-9]+,[0-9]+ \+[0-9]+,[0-9]+ @@$", text), text
        else:
            assert "```diff" not in text, f"default export unexpectedly quotes code:\n{text}"
        tui.quit()
