#!/usr/bin/env python3
# A repository `.mailmap` is respected when resolving commit authors (git),
# for both the default libgit2 backend and the `backend = "cli"` backend.
import shutil

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    # All commits were made as E2E <e2e@example.invalid>; the mailmap rewrites
    # that identity to "Mapped <mapped@example.invalid>".
    repos.write(
        repo / ".mailmap",
        "Mapped <mapped@example.invalid> E2E <e2e@example.invalid>\n",
    )
    repos.git(repo, "add", ".mailmap")
    repos.git(repo, "commit", "-qm", "add mailmap")

    # Review commits made before `.mailmap` was added (`HEAD~3..HEAD~1` =
    # the two middle commits). `.mailmap` is present in the worktree so their
    # authors resolve to "Mapped", but none of these commits touch `.mailmap`,
    # so "Mapped" appears only in the commit selector's author column — never
    # in diff content. That makes `wait_for("Mapped")` a reliable assertion.
    tui = s.start("-r", "HEAD~3..HEAD~1", cwd=repo)
    tui.wait_for("Mapped")  # inline selector lists mapped commit authors
    tui.quit()

    # `backend = "cli"` backend uses `%aN` in `git log`/`git show`, which respects `.mailmap`.
    repos.write(
        s.dir / "config" / "tuicr" / "config.toml",
        'backend = "cli"\n',
    )
    # keep `config/` so the override survives; drop runtime dirs for a fresh start
    for sub in ("home", "data", "state", "tmp", "work"):
        shutil.rmtree(s.dir / sub, ignore_errors=True)
    tui = s.start("-r", "HEAD~3..HEAD~1", cwd=repo, keep=True)
    tui.wait_for("Mapped")
    tui.quit()
