#!/usr/bin/env python3
# A repository `.mailmap` is respected when resolving commit authors (git),
# for both the default libgit2 backend and the `backend = "cli"` backend.
from e2e import Scenario, flows, repos, scenario


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
    git = flows.GitCalls(s)

    # Review commits made before `.mailmap` was added (`HEAD~3..HEAD~1` = the two middle commits). `.mailmap` is
    # present in the worktree so their authors resolve to "Mapped", but none of these commits touch `.mailmap`, so
    # "Mapped" (and the raw "E2E") can appear only in the commit selector's author column, never in diff content.
    def authors_mapped(name: str, cli: bool) -> None:
        tui = s.start("-r", "HEAD~3..HEAD~1", cwd=repo, name=name, keep=True)
        tui.wait_for("Files · 0/8").wait_for("Mapped")
        assert git.cli_backend(name) == cli, f"{name}: wrong backend; git calls: {git.calls(name)}"
        rows = [ln for ln in tui.screen().split("\n") if "Mapped" in ln]
        for subject in ("fix: greeting", "feat: touch every file kind"):
            assert any(subject in row for row in rows), f"{name}: {subject!r} is not shown as Mapped:\n{tui.screen()}"
        tui.reject(r"\bE2E\b")  # the raw identity
        tui.quit()

    authors_mapped(s.name, cli=False)
    # `backend = "cli"` uses `%aN` in `git log`/`git show`, which respects `.mailmap`.
    cli = f"{s.name}_cli"
    repos.write(s.path_of(cli) / "config/tuicr/config.toml", 'backend = "cli"\n')
    authors_mapped(cli, cli=True)
