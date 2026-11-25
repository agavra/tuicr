#!/usr/bin/env python3
# Covers #252: a Git revision review includes explicit untracked drafts and refreshes their text and comments.
from e2e import Scenario, grep, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    draft = repos.write(repo / ".drafts/description.md", "Original draft description\n")
    repos.write(repo / ".git/info/exclude", "/.drafts/\n")
    repos.write(repo / "notes-draft.txt", "Additional draft notes\n")

    tui = s.start("-r", "HEAD~2..HEAD", "--file", ".drafts/description.md", "--file", "notes-draft.txt", cwd=repo)
    tui.wait_for("Files · 0/10").keys("Escape")
    tui.cursor_on("4 ▌ second line").comment("Code review comment")
    tui.cursor_on("1 ▌ Original draft description").comment("Draft review comment")
    tui.wait_for("Additional draft notes")

    repos.write(draft, "Updated draft description\n")
    tui.cmd("e").wait_for("Updated draft description").wait_for("Draft review comment")
    tui.wait_gone("Original draft description")

    output = s.dir / "review.md"
    tui.cmd(f"file {output}").wait_for("Review written to")
    markdown = output.read_text()
    assert grep(r"`README\.md:4` - Code review comment", markdown), markdown
    assert grep(r"`\.drafts/description\.md:1` - Draft review comment", markdown), markdown
    tui.quit()

    tracked = s.cli_result("-r", "HEAD~2..HEAD", "--file", "src/main.rs", cwd=repo)
    assert tracked.rc != 0 and "tracked" in tracked.out and "src/main.rs" in tracked.out, tracked.out
    outside = repos.write(s.dir / "outside.txt", "outside the checkout\n")
    external = s.cli_result("-r", "HEAD~2..HEAD", "--file", str(outside), cwd=repo)
    assert external.rc != 0 and "outside" in external.out and "checkout" in external.out, external.out
