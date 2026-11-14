"""Fake `glab` for MR !1 of gitlab.com/octo/demo. Fixtures: data/gitlab/.

Stateful like real GitLab: POSTed draft notes are kept in the run's state dir and served back
ONLY by GET .../draft_notes (drafts are hidden from /discussions).
POSTs of published notes/discussions, approve and the request-changes GraphQL mutation are only logged.
Knobs (env): E2E_GL_DISCUSSIONS=<json file> served as /discussions page 1;
E2E_GL_DIFFS=<json file> replaces the MR diffs; E2E_GL_FAIL_FLAG=<file>: while it exists a
draft_notes POST fails once one draft is stored; E2E_GL_FILES=<dir>: raw file contents
(default: data/github/contents).
"""

from __future__ import annotations

import json
import os
import re
import urllib.parse

from .core import Fake

PROJECT = r"projects/octo%2Fdemo/(.*)"
DRAFTS = "gl_drafts.json"


def handle(f: Fake) -> None:
    argv = f.argv
    f.start(read_stdin="--input" in argv)
    if argv[:2] == ["config", "get"]:  # no configured host / subfolder
        f.out("")
    if argv[:1] != ["api"]:
        f.unexpected("cmd " + " ".join(argv))
    ep = f.positional(argv[1:], ("--hostname", "--method", "--header", "--input", "-f"))[0]
    method = f.flag("--method") or "GET"
    if ep == "graphql":  # mergeRequestRequestChanges mutation (request-changes submit)
        f.out({"data": {"mergeRequestRequestChanges": {"errors": []}}})
    base, _, query = ep.partition("?")
    first_page = f.page(query) == "1"
    if base == "user":
        f.serve("user.json")
    m = re.fullmatch(PROJECT, base)
    if not m:
        f.unexpected("endpoint " + ep)
    rest = m.group(1)
    if rest == "merge_requests/1":
        f.serve("mr.json")
    if rest == "merge_requests/1/diffs":
        diffs = os.environ.get("E2E_GL_DIFFS")
        f.out(open(diffs).read() if diffs else f.load("mr_diffs.json")) if first_page else f.out("[]")
    if rest == "merge_requests/1/commits":
        f.serve("mr_commits.json") if first_page else f.out("[]")
    if rest == "merge_requests/1/versions":
        f.out("[]")
    if rest == "merge_requests/1/approvals":
        f.serve("mr_approvals.json")
    if rest == "merge_requests/1/discussions":
        disc = os.environ.get("E2E_GL_DISCUSSIONS")
        f.out(open(disc).read() if disc and first_page else "[]")
    if method == "POST" and rest in ("merge_requests/1/notes", "merge_requests/1/discussions"):
        f.out(
            {"id": "9f" * 10 if rest.endswith("discussions") else 7}
        )  # published review pieces: logged, not served back
    if method == "POST" and rest == "merge_requests/1/approve":
        f.out({})
    if rest == "merge_requests/1/draft_notes":
        return draft_notes(f, method, first_page)
    if rest.startswith(("repository/files/", "repository/commits")):
        d = os.environ.get("E2E_GL_FILES") or os.path.join(os.environ["E2E_DATA"], "github", "contents")
        mm = re.match(r"repository/files/([^/]+)/raw", rest)
        if mm:
            p = os.path.join(d, urllib.parse.unquote(mm.group(1)))
            if os.path.exists(p):
                f.out(open(p).read())
        f.unexpected("file/commit lookup " + ep)
    f.unexpected("rest " + ep)


def draft_notes(f: Fake, method: str, first_page: bool) -> None:
    drafts = f.state_json(DRAFTS, [])
    if method == "POST":
        flag = os.environ.get("E2E_GL_FAIL_FLAG")
        if flag and os.path.exists(flag) and drafts:
            f.record("injected failure")
            f.fail("glab: 500 Internal Server Error (injected)")
        note = json.loads(f.stdin or "")
        note["id"] = len(drafts) + 1
        drafts.append(note)
        f.save_state(DRAFTS, drafts)
        f.out({"id": note["id"]})
    f.out(drafts if first_page else [])
