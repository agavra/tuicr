"""Fake `tea api -i` for PR #1 of gitea.example.com/octo/demo. Fixtures: data/gitea/.

Like `tea`, every response is "HTTP 200" at exit 0: the status line + headers go to stderr, the body to stdout.
Stateful: POSTed reviews are kept in the run's state dir; no event => PENDING (like Gitea).
Knobs (env): E2E_GT_ROWS rows of PR list in total, server caps `limit` at E2E_GT_CAP (like MaxResponseItems).
"""
import json
import os
import re
import sys

HEAD = "a" * 40
BASE = "b" * 40
PULL = "/repos/octo/demo/pulls/1"
REVIEWS = "gt_reviews.json"


def reply(f, body, status="200 OK", total=None):
    sys.stderr.write(f"HTTP/1.1 {status}\r\n" + (f"X-Total-Count: {total}\r\n" if total is not None else "") + "\r\n")
    f.out(body)


def handle(f):
    argv = f.argv
    f.start(read_stdin="@-" in argv)
    if argv[:2] == ["logins", "list"]:
        f.out("[]")
    if argv[:2] != ["api", "-i"]:
        f.unexpected("unsupported tea command")
    method = f.flag("--method") or "GET"
    ep = argv[-1]
    base, _, query = ep.partition("?")
    page = f.page(query)
    first_page = page == "1"
    reviews = f.state_json(REVIEWS, [])
    if base == "/user":
        reply(f, f.load("user.json"))
    if base == "/repos/octo/demo/pulls" and method == "GET":
        return pr_list(f, query, int(page))
    if base == PULL:
        reply(f, f.load("pr.json"))
    if base == PULL + "/files":
        reply(f, f.load("pr_files.json") if first_page else "[]", total=1)
    if base == PULL + ".diff":
        reply(f, f.load("pr.diff"))
    if base == PULL + "/commits":
        reply(f, f.load("pr_commits.json") if first_page else "[]", total=1)
    if base == PULL + "/reviews":
        if method == "POST":
            return post_review(f, reviews)
        reply(f, reviews if first_page else [], total=len(reviews))
    if re.fullmatch(PULL + r"/reviews/(\d+)/comments", base) or base == "/repos/octo/demo/issues/1/comments":
        reply(f, "[]")
    if base.startswith("/repos/octo/demo/raw/"):
        p = os.path.join(os.environ["E2E_DATA"], "github", "contents", base[len("/repos/octo/demo/raw/"):])
        if os.path.exists(p):
            reply(f, open(p).read())
    f.record("UNEXPECTED " + ep)
    reply(f, '{"message":"no route"}', "404 Not Found")


def pr_list(f, query, page):
    total = int(os.environ.get("E2E_GT_ROWS", "1"))
    cap = int(os.environ.get("E2E_GT_CAP", "50"))
    m = re.search(r"(?:^|&)limit=(\d+)", query)
    lim = min(int(m.group(1) if m else "30"), cap)
    rows = [{"number": i, "title": "GT row %d" % i, "state": "open", "user": {"login": "alice"},
             "html_url": "https://gitea.example.com/octo/demo/pulls/%d" % i, "head": {"ref": "f%d" % i, "sha": HEAD},
             "base": {"ref": "main", "sha": BASE}, "updated_at": "2026-01-02T03:04:05Z", "draft": False}
            for i in range(1, total + 1)]
    reply(f, rows[(page - 1) * lim: page * lim], total=total)


def post_review(f, reviews):
    p = json.loads(f.stdin)
    n = len(reviews) + 1
    r = {"id": n, "state": "COMMENT" if p.get("event") else "PENDING", "body": p.get("body", ""), "user": {"login": "me"},
         "comments_count": len(p.get("comments", [])), "commit_id": p.get("commit_id"),
         "html_url": "https://gitea.example.com/octo/demo/pulls/1#r%d" % n}
    reviews.append(r)
    f.save_state(REVIEWS, reviews)
    reply(f, r)
