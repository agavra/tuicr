#!/usr/bin/env python3
# Covers #890 / #149: :file writes the same complete Markdown as :clip and replaces an existing file.
from e2e import Scenario, grep, scenario, wait_file


@scenario
def main(s: Scenario) -> None:
    clipboard = s.fake_clipboard()
    tui = s.diff()
    tui.cursor_on("4 ▌ second line")
    tui.comment("write this review to a file")
    output = s.dir / "review with spaces.md"
    tui.cmd(f"file {output}")
    written = wait_file(output).read_text()
    assert grep(r"^1\. `README\.md:4` - write this review to a file$", written), written
    tui.cmd("clip").wait_for("Review copied to clipboard")
    assert written == clipboard.text(), "file differs from the clipboard export"
    output.write_text("stale contents that must be replaced")
    tui.cmd(f"file {output}").wait_for("Review written to")
    assert output.read_text() == written, "existing file was not replaced with the complete export"
    tui.quit()
