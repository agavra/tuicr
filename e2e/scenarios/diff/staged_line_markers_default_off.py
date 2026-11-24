#!/usr/bin/env python3
# Covers #883 / #697: the default and explicit false keep the gutter and skip marker-only diff reads.
from e2e import Scenario, repos, scenario, wait_grep


@scenario
def main(s: Scenario) -> None:
    repo = repos.git_repo(s.dir / "repo")
    path = repo / "README.md"
    original = path.read_text()

    for setting, config in (("default", ""), ("disabled", "staged_markers = false\n")):
        name = f"{s.name}_{setting}"
        directory = s.path_of(name)
        repos.write(directory / "config/tuicr/config.toml", f"diff_watch_interval_ms = 200\n{config}")
        repos.write(path, original)
        profile = directory / "profile.log"
        tui = s.start("-w", cwd=repo, name=name, keep=True, env={"TUICR_PROFILE": str(profile)})
        tui.wait_for("unstaged line")
        tui.reject(r"\d[●○].*unstaged line")

        repos.write(path, f"{original}\nupdated during the review\n")
        tui.wait_for("updated during the review")
        tui.reject(r"\d[●○].*updated during the review")
        tui.quit()

        # This fixture needs no full staged/unstaged loads for its status rows.
        wait_grep(profile, r'diff\.load_working_tree"')
        operations = profile.read_text()
        assert 'diff.load_staged"' not in operations, f"{setting} loaded the staged diff for markers"
        assert 'diff.load_unstaged"' not in operations, f"{setting} loaded the unstaged diff for markers"
