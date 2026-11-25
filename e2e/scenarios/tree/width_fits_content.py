#!/usr/bin/env python3
# Covers #770: the file tree is as wide as its widest row, capped at a fifth of the terminal.
# Baseline always gave it a fifth, so at 160 columns two short paths sat in a 32-column panel.
from e2e import Scenario, because, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.empty_repo(s.dir / "repo")
    repos.write(repo / "notes.txt", "one\n")
    repos.write(repo / "src/medium_name.rs", "one\n")
    repos.git(repo, "add", "-A")
    repos.git(repo, "commit", "-qm", "base")
    repos.write(repo / "notes.txt", "two\n")
    repos.write(repo / "src/medium_name.rs", "two\n")
    tui = s.start("-w", cwd=repo)
    # `  ▢ M medium_name.rs` is 20 columns; one column of space and both borders make 23, so 8 rules follow the
    # 13-column title and the diff's border starts right after.
    with because("file tree not sized to its widest row"):
        tui.wait_for(r"^┌ Files · 0/2 ─{8}┐┌")
    # A fifth of 80 columns is 16, narrower than the row: the cap wins.
    tui.resize(80, 45)
    with because("file tree not capped at a fifth of the terminal"):
        tui.wait_for(r"^┌ Files · 0/2 ─┐┌")
    tui.quit()
