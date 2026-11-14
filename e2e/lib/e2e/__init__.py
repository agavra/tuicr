"""Python e2e library for tuicr: a tmux terminal driver, waits, fixtures, fake forges, one function per scenario.

    from e2e import scenario

    @scenario
    def main(s):                      # s: Scenario
        tui = s.diff()                # fixture repo + tuicr on the diff; tui: Tui
        tui.keys("c").wait_for("COMMENT")
        tui.quit()

See e2e/README.md ("Writing a scenario") for the API.
"""
from .core import (E2E_DIR, OUT, ROOT, TUICR_BIN, CliResult, Failure, NoPane, Scenario, Skip, because, grep, must_fail, scenario)
from .tui import Tui, tmux
from .waits import wait_cmd, wait_file, wait_grep
from . import flows, forge, repos
from .forge import pr_comment

__all__ = ["E2E_DIR", "OUT", "ROOT", "TUICR_BIN", "CliResult", "Failure", "because", "NoPane", "Scenario", "Skip", "Tui", "flows",
           "forge", "grep", "must_fail", "pr_comment", "repos", "scenario", "tmux", "wait_cmd", "wait_file", "wait_grep"]
