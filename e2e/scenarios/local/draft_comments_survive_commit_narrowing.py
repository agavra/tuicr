#!/usr/bin/env python3
# Covers #998: a comment on a `--file` draft made while one commit is selected stays visible on other commits,
# because the draft itself stays visible across every commit selection.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    repos.write(repo / "notes-draft.txt", "Draft notes\n")
    repos.write(s.dir / "config" / "tuicr" / "config.toml", 'initial_commit_selection = "oldest"\n')

    tui = s.start("-r", "HEAD~2..HEAD", "--file", "notes-draft.txt", cwd=repo, keep=True)
    tui.wait_for(r"Commit Message \(").keys("Escape")
    tui.cursor_on("1 ▌ Draft notes").comment("Draft review comment")
    tui.wait_for("Draft review comment")

    tui.keys("(").settle()  # oldest commit → the newer one
    tui.expect(r"Commit Message \(").expect("Draft notes")
    tui.expect("Draft review comment")  # hidden on the next commit when stamped with the oldest commit's SHA
    tui.quit()
