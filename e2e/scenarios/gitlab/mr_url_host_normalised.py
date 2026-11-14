#!/usr/bin/env python3
# Covers #910: a GitLab MR URL with an upper-case host, a port and userinfo, in a subgroup, must reach glab with the
# bare host.
# Baseline kept userinfo and port in the host it handed to glab (`--hostname me@GitLab.Example.com:8443`), which no glab
# configuration matches; the fix passes the bare hostname.
import json
import os
import re

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.fresh(s.dir / "repo")
    repos.run(repo, "git", "init", "-q", ".")
    os.chdir(repo)
    log = s.dir / "forge.log"
    log.unlink(missing_ok=True)
    # The fake glab has no fixture for this project, so the open fails: only the host handed to glab is asserted.
    s.allow_unexpected_forge_calls("no fixture for this project: the open fails on purpose")
    r = s.cli_result(
        "--no-update-check",
        "pr",
        "https://me@GitLab.Example.com:8443/grp/sub/proj/-/merge_requests/7",
    )
    print("\n".join(r.out.splitlines()[:5]))
    assert r.rc != 0, "tuicr pr unexpectedly succeeded against a forge without fixtures"
    calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
    print("--- forge calls:")
    for c in calls:
        print(c["tool"], " ".join(c["argv"])[:120])
    assert any(c["tool"] == "glab" for c in calls), "glab was never called"
    # the host handed to glab must be the bare hostname: baseline passed `me@GitLab.Example.com:8443` (userinfo + port)
    hosts = [c["argv"][i + 1] for c in calls for i, a in enumerate(c["argv"][:-1]) if a == "--hostname"]
    assert not any(re.search("[@:]", h) for h in hosts), f"glab was called with userinfo/port in --hostname: {hosts}"
    assert "GitLab.Example.com" in hosts, f"no glab call with the bare host: {hosts}"
