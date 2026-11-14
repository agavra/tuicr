"""Fake `bkt` for PR #1 of bitbucket.org/octo/demo. Fixtures: data/bitbucket/.

The diff and file contents are the GitHub fixtures' (README.md + src/greet.rs).
POSTed comments, `pr approve` and request-changes are only logged (the body is the `--input` argument).
Knob (env): E2E_BB_COMMENTS=<json file>: the list served for /comments.
"""

from __future__ import annotations

import json
import os

from .core import Fake

HEAD = "2" * 40
REPO = "/2.0/repositories/octo/demo"


def handle(f: Fake) -> None:
    argv = f.argv
    f.start()
    github = os.path.join(os.environ["E2E_DATA"], "github")
    if argv[:2] == ["pr", "diff"]:
        f.out(open(os.path.join(github, "pr_diff.patch")).read())
    if argv[:3] == ["pr", "approve", "1"]:  # participant state change: logged only
        f.out("")
    if argv[:1] != ["api"]:
        f.unexpected(" ".join(argv))
    ep = argv[1].split("?")[0]
    pr = REPO + "/pullrequests/1"
    if f.flag("--method") == "POST":  # published comments / request-changes: logged, not served back
        if ep == pr + "/comments":
            f.out({"id": 4000 + sum(1 for _ in open(f.log or ""))})
        if ep == pr + "/request-changes":
            f.out({})
    if ep == "/2.0/user":
        f.serve("user.json")
    if ep == pr:
        f.serve("pr.json")
    if ep == pr + "/diffstat":
        f.serve("diffstat.json")
    if ep == pr + "/commits":
        f.serve("commits.json")
    if ep == pr + "/comments":
        path = os.environ.get("E2E_BB_COMMENTS")
        f.out({"values": json.load(open(path)) if path else []})
    if ep.startswith((REPO + "/merge-base/", REPO + "/commit/")):
        f.fail("404")
    if ep.startswith(REPO + "/src/"):
        p = os.path.join(github, "contents", ep.split("/src/" + HEAD + "/", 1)[-1])
        if os.path.exists(p):
            f.out(open(p).read())
        f.fail("404")
    f.unexpected("endpoint " + ep)
