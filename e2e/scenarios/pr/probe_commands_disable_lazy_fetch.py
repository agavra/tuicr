#!/usr/bin/env python3
# Covers #959: local object probes must not trigger promisor fetches, or `tuicr pr`
# stalls for minutes in a blob:none partial clone before the forge fallback runs.
import shlex
import shutil

from e2e import Scenario, repos, scenario
from e2e.core import E2E_DIR, TOOL_PATH


@scenario
def main(s: Scenario) -> None:
    repo = repos.pr_repo(s.dir / "repo")
    bindir = s.dir / "bin"
    bindir.mkdir(parents=True, exist_ok=True)
    log = s.dir / "probe.log"
    real_git = shutil.which("git")
    assert real_git is not None, "git not found"
    wrapper = bindir / "git"
    wrapper.write_text(
        "#!/bin/sh\n"
        'case "$1" in\n'
        "  cat-file|merge-base)\n"
        f'    printf \'%s %s\\n\' "$1" "${{GIT_NO_LAZY_FETCH:-unset}}" >> {shlex.quote(str(log))}\n'
        "    ;;\n"
        "esac\n"
        f'exec {shlex.quote(real_git)} "$@"\n'
    )
    wrapper.chmod(0o755)

    tui = s.start("pr", "1", cwd=repo, env={"PATH": f"{bindir}:{E2E_DIR}/bin:{TOOL_PATH}"})
    tui.wait_for("Add greeting and tweak README")
    tui.keys(":q", "Enter")
    tui.wait_exit()

    probes = log.read_text().splitlines()
    assert probes, "no cat-file/merge-base probes were run"
    unguarded = [line for line in probes if not line.endswith(" 1")]
    assert not unguarded, f"probes without GIT_NO_LAZY_FETCH=1: {unguarded}"
