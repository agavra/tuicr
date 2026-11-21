"""Fake `glab` for MR !1 of gitlab.com/octo/demo. Fixtures: data/gitlab/.

Stateful like real GitLab: a POSTed draft note is stored in GitLab's own draft-note shape (id, author_id,
merge_request_id, note, discussion_id, position with every coordinate, null when unset), returned as the POST's answer
and served back ONLY by GET .../draft_notes (drafts are hidden from /discussions). A POST without `note` is refused.
POSTs of published notes/discussions, approve and the request-changes GraphQL mutation (for octo/demo !1 only; any
other GraphQL call is unexpected) are only logged.
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
USER_ID, MR_ID = 4, 11  # the reviewer's user id and MR !1's global id (GitLab returns both on a draft note)
POSITION_KEYS = (
    "base_sha",
    "start_sha",
    "head_sha",
    "old_path",
    "new_path",
    "position_type",
    "old_line",
    "new_line",
    "line_range",
)


def handle(f: Fake) -> None:
    argv = f.argv
    f.start(read_stdin="--input" in argv)
    if argv[:2] == ["config", "get"]:  # no configured host / subfolder
        f.out("")
    if argv[:1] != ["api"]:
        f.unexpected("cmd " + " ".join(argv))
    ep = f.positional(argv[1:], ("--hostname", "--method", "--header", "--input", "-f"))[0]
    method = f.flag("--method") or "GET"
    if ep == "graphql":  # only the request-changes mutation, for MR !1 of octo/demo
        fields = dict(argv[i + 1].split("=", 1) for i in range(len(argv) - 1) if argv[i] == "-f" and "=" in argv[i + 1])
        if (
            fields.get("query", "").startswith("mutation")
            and "mergeRequestRequestChanges(" in fields["query"]
            and (fields.get("projectPath"), fields.get("iid")) == ("octo/demo", "1")
        ):
            f.out({"data": {"mergeRequestRequestChanges": {"errors": []}}})
        f.unexpected("graphql " + json.dumps(fields))
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


def draft_note(req: dict, note_id: int) -> dict:
    """What GitLab stores and returns for a POSTed draft note: its own fields, not an echo of the request. A note
    without a position (a review-level draft) gets a position object whose coordinates are all null."""
    pos = req.get("position") or {}
    return {
        "id": note_id,
        "author_id": USER_ID,
        "merge_request_id": MR_ID,
        "resolve_discussion": bool(req.get("resolve_discussion", False)),
        "discussion_id": req.get("in_reply_to_discussion_id"),
        "note": req["note"],
        "commit_id": req.get("commit_id"),
        "line_code": None,
        "position": {k: pos.get(k) for k in POSITION_KEYS} | {"position_type": pos.get("position_type") or "text"},
    }


def draft_notes(f: Fake, method: str, first_page: bool) -> None:
    drafts = f.state_json(DRAFTS, [])
    if method == "POST":
        flag = os.environ.get("E2E_GL_FAIL_FLAG")
        if flag and os.path.exists(flag) and drafts:
            f.record("injected failure")
            f.fail("glab: 500 Internal Server Error (injected)")
        req = json.loads(f.stdin or "null")
        if not isinstance(req, dict) or not isinstance(req.get("note"), str) or not req["note"]:
            f.unexpected("draft note POST without a note: " + (f.stdin or ""))
        note = draft_note(req, 100 + len(drafts))
        drafts.append(note)
        f.save_state(DRAFTS, drafts)
        f.out(note)
    f.out(drafts if first_page else [])
