#!/usr/bin/env python3
# `tuicr --help` and `tuicr pr --help` name every supported forge instead of only GitHub/GitLab/Azure DevOps
# (clap help text undersold Gitea, Bitbucket and Gerrit).
from e2e import Scenario, scenario

FORGES = ["GitHub", "GitLab", "Heptapod", "Gitea", "Bitbucket", "Azure DevOps", "Gerrit"]


@scenario
def main(s: Scenario) -> None:
    results = {}
    for args in [["--help"], ["pr", "--help"]]:
        r = results[" ".join(args)] = s.cli_result(*args)
        assert r.rc == 0, f"'tuicr {' '.join(args)}' exited {r.rc}. Output: {r.out}"
        missing = [f for f in FORGES if f not in r.out]
        assert not missing, f"'tuicr {' '.join(args)}' omits {missing}. Output: {r.out}"
        print(f"ok: tuicr {' '.join(args)} names every forge")
    r = results["--help"]
    assert "Review a pull request, merge request, or Gerrit change" in r.out, f"pr subcommand summary stale: {r.out}"
