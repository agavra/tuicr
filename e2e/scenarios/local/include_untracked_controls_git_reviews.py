#!/usr/bin/env python3
# Covers #431/#695: both Git backends honor include_untracked in working-tree, combined and selector reviews.
from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    for backend in ("libgit2", "cli"):
        for enabled in (True, False):
            for mode in ("worktree", "combined", "selector"):
                name = f"{s.name}_{backend}_{enabled}_{mode}"
                config = f'backend = "{backend}"\ndiff_watch_interval_ms = 50\n'
                if not enabled:
                    config += "include_untracked = false\n"
                repos.write(s.path_of(name) / "config/tuicr/config.toml", config)
                args = ("-r", "HEAD", "-w") if mode == "combined" else ("-w",) if mode == "worktree" else ()
                tui = s.start(*args, cwd=repo, name=name, keep=True)
                if mode == "selector":
                    tui.wait_for("Unstaged changes").expect("Staged changes").keys("j", "Enter")
                    tui.expect(r"▢ A staged\.txt").expect("staged addition").reject(r"untracked\.txt")
                    tui.keys("Escape").cmd("commits").wait_for("Unstaged changes").keys("Enter")
                tui.expect(r"▢ M README\.md").expect("unstaged line")
                if enabled:
                    tui.expect(r"▢ A untracked\.txt")
                else:
                    tui.reject(r"untracked\.txt")
                if mode == "worktree":
                    # The watch worker reopens the backend with the same option.
                    marker = f"watch {backend} {enabled}"
                    repos.write(repo / "README.md", f"# demo\n\nfirst line\nunstaged line\n{marker}\n")
                    tui.expect(marker)
                    if not enabled:
                        tui.reject(r"untracked\.txt")
                tui.quit()
