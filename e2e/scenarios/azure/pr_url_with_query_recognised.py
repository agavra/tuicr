#!/usr/bin/env python3
# Covers #897: an Azure DevOps PR URL with a query (`?_a=files`) is recognised as an Azure target. Without PAT / az
# the open stops at the Azure auth error (no network needed). Baseline fell through to the Gerrit parser:
# "Malformed Gerrit change target".
import os

from e2e import Scenario, repos, scenario


@scenario
def main(s: Scenario) -> None:
    repo = repos.fresh(s.dir / "repo")
    repos.run(repo, "git", "init", "-q", ".")
    os.chdir(repo)
    # Deterministic: only git on PATH, so `az` is absent on every machine (CI images ship the real Azure CLI).
    minbin = s.minimal_path(s.dir / "minbin", "git")
    for url in (
        "https://dev.azure.com/myorg/myproject/_git/myrepo/pullrequest/42",
        "https://dev.azure.com/myorg/myproject/_git/myrepo/pullrequest/42?_a=files#discussion",
    ):
        r = s.cli_result("--no-update-check", "pr", url, path=minbin)
        print(f"{url} -> {r.out}")
        assert r.rc != 0, "tuicr pr unexpectedly succeeded"
        assert "Azure DevOps integration needs" in r.out, f"Azure PR URL not recognised: {url}"
