#!/usr/bin/env python3
# Covers #431/#695: disabling untracked files keeps the selector rows empty and preserves staged files on unborn HEAD.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    for backend in ("libgit2", "cli"):
        name = f"{s.name}_{backend}"
        repo = repos.empty_repo(s.dir / f"repo_{backend}")
        repos.write(s.path_of(name) / "config/tuicr/config.toml", f'backend = "{backend}"\ninclude_untracked = false\n')
        repos.write(repo / "tracked.txt", "initial content\n")
        repos.git(repo, "add", "tracked.txt")
        repos.git(repo, "commit", "-qm", "initial")
        repos.write(repo / "tracked.txt", "second commit\n")
        repos.git(repo, "commit", "-qam", "second")
        repos.write(repo / "untracked.txt", "excluded\n")
        tui = s.start(cwd=repo, name=name, keep=True)
        tui.wait_for("second").expect(r"Unstaged changes.*\(no changes\)").expect(r"Staged changes.*\(no changes\)")
        tui.keys("Enter").expect("second commit").reject(r"untracked\.txt").quit()
        repo = repos.empty_repo(s.dir / f"unborn_{backend}")
        repos.write(repo / "new.txt", "untracked content\n")
        for args in ((), ("-w",)):
            result = s.cli_result("--no-update-check", *args, cwd=repo, name=name)
            assert result.rc == 1 and "No changes to review" in result.out, result.out
        repos.git(repo, "add", "new.txt")
        tui = s.start(cwd=repo, name=name, keep=True)
        tui.wait_for("Staged changes").expect(r"Unstaged changes.*\(no changes\)")
        tui.reject(r"Staged changes.*\(no changes\)").keys("Enter")
        tui.expect(r"▢ A new\.txt").expect("untracked content").quit()
