#!/usr/bin/env python3
# `--theme <unknown>` fails at startup (exit 2, lists the bundled themes) without opening the TUI; a valid `--theme`
# beats an unknown `theme` in config.toml (the TUI shows the same colours as `--theme nord-dark` with no config).
from e2e import Scenario, Tui, grep, repos, scenario

ROWS = (1, 40)  # header, file tree and diff; not the status line (it may carry a startup warning)


def nord_colours(s: Scenario, name: str, config: str | None) -> int:
    if config is not None:
        repos.write(s.path_of(name) / "config" / "tuicr" / "config.toml", config)
    tui: Tui = s.start("--theme", "nord-dark", "-r", "HEAD~2..HEAD", cwd=s.dir / "repo", name=name, keep=True)
    h = tui.wait_for("Files · 0/8").keys("Escape").settle().colors_hash(ROWS)
    tui.quit()
    return h


@scenario
def main(s: Scenario) -> None:
    (s.dir / "config" / "tuicr").mkdir(parents=True)
    repo = repos.git_repo(s.dir / "repo")
    res = s.cli_result("--no-update-check", "--theme", "nosuch-theme", cwd=repo)
    assert res.rc == 2, f"exit status {res.rc}, expected 2\n{res.out}"
    assert grep("Unknown theme 'nosuch-theme'", res.out) and grep("nord-dark", res.out), (
        f"no theme error with the bundled list:\n{res.out}"
    )
    with_bad_config = nord_colours(s, s.name, 'theme = "also-missing"\n')
    reference = nord_colours(s, s.name + "_ref", None)
    # control: without --theme the same screen looks different, so equal hashes above mean nord-dark won
    plain = s.start("-r", "HEAD~2..HEAD", cwd=repo, name=s.name + "_plain")
    default = plain.wait_for("Files · 0/8").keys("Escape").settle().colors_hash(ROWS)
    plain.quit()
    assert default != reference, "precondition: nord-dark colours the screen like the default theme"
    assert with_bad_config == reference, "--theme nord-dark did not win over the unknown theme in config.toml"
