"""Fake `gh`: PR open/list, REST (files, commits, compare, contents, reviews), GraphQL.

Fixtures: data/github/. Knobs (env): E2E_VARIANT / E2E_VARIANT_FILE (overlay dir),
E2E_LIST_ROWS / E2E_LIST_FAIL_ABOVE (PR list size, failing load-more),
E2E_HEAD_MODE=once|every (PR head moves while the PR opens), E2E_HOLD_DIR (core).
"""
import json
import os
import re
import sys
import urllib.parse

API_VALUE_FLAGS = ("-H", "--hostname", "--jq", "-f", "-F", "--method", "--input", "-X")
EMPTY_THREADS = {"data": {"repository": {"pullRequest": {"reviewThreads": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": []}}}}}
EMPTY_REVIEWS = {"data": {"repository": {"pullRequest": {"reviews": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": []}}}}}
EMPTY_METADATA = {"data": {"viewer": {"login": "reviewer"}, "repository": {"pullRequest": {"reviews": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": []}}}}}
REVIEW_CREATED = '{"id": 1001, "html_url": "https://github.com/octo/demo/pull/1#pullrequestreview-1001", "state": "COMMENTED"}'


def handle(f):
    argv = f.argv
    f.start(read_stdin="--input" in argv and f.flag("--input") == "-")
    if argv[:2] == ["pr", "list"]:
        return pr_list(f)
    if argv[:2] == ["pr", "view"]:
        return pr_view(f)
    if argv[:2] == ["pr", "diff"]:
        f.serve("pr_diff.patch")
    if argv[:1] != ["api"]:
        f.unexpected("unsupported gh command")
    if "--jq" in argv:  # tuicr's fork-parent probe: `api repos/o/r --jq {parent:...}`
        if re.fullmatch(r"repos/[^/]+/[^/]+", argv[1]):
            f.serve("repo_parent.json", '{"parent":null}\n')
        f.unexpected("unsupported --jq call")
    fields = {}
    for i, a in enumerate(argv):
        if a in ("-f", "-F") and "=" in argv[i + 1]:
            k, v = argv[i + 1].split("=", 1)
            fields[k] = v
    pos = f.positional(argv[1:], API_VALUE_FLAGS)
    if pos == ["graphql"]:
        return graphql(f, fields)
    return rest(f, pos[0] if pos else "")


def pr_list(f):
    if not os.environ.get("E2E_LIST_ROWS"):
        f.serve("pr_list.json")
    # E2E_LIST_ROWS=N: serve N rows (truncated to --limit); E2E_LIST_FAIL_ABOVE=M: --limit > M fails (a failing load-more)
    limit = int(f.flag("--limit") or 30)
    above = os.environ.get("E2E_LIST_FAIL_ABOVE")
    if above and limit > int(above):
        f.fail("simulated network failure")
    base = f.load_json("pr_list.json")[0]
    rows = [dict(base, number=i, title="Row %d %s" % (i, "alpha" if i % 2 else "beta"), url="https://github.com/octo/demo/pull/%d" % i)
            for i in range(1, int(os.environ["E2E_LIST_ROWS"]) + 1)]
    f.out(json.dumps(rows[:limit]) + "\n")


def data_calls_so_far(f):
    """Number of PR data calls (files/commits/diff) already logged: drives E2E_HEAD_MODE."""
    n = 0
    if f.log and os.path.exists(f.log):
        for line in open(f.log):
            try:
                a = json.loads(line)["argv"]
            except Exception:
                continue
            if any(x.startswith(("repos/octo/demo/pulls/1/files", "repos/octo/demo/pulls/1/commits")) or x in ("pulls/1/files", "pulls/1/commits") for x in a) or a[:2] == ["pr", "diff"]:
                n += 1
    return n


def pr_view(f):
    mode = os.environ.get("E2E_HEAD_MODE")
    if not mode:
        f.serve("pr_view.json")
    # once: head A (2222..) until the first data call, then 4444..; every: a new head after every data call (a push racing PR open)
    n = data_calls_so_far(f)
    body = f.load_json("pr_view.json")
    if mode == "once":
        body["headRefOid"] = "2" * 40 if n == 0 else "4" * 40
    else:
        body["headRefOid"] = ("%x" % (n % 16)) * 40 if n else "2" * 40
    f.out(json.dumps(body) + "\n")


def graphql(f, fields):
    q = fields.get("query", "")
    if "committedDate" in q:
        repo = {f"pr{n}": {"commits": {"nodes": [{"commit": {"committedDate": "2026-01-02T03:04:05Z"}}]}}
                for n in re.findall(r"pr(\d+): pullRequest", q)}
        f.out(json.dumps({"data": {"repository": repo}}) + "\n")
    if "object(expression" in q:
        repo = {}
        for k, v in fields.items():
            m = re.fullmatch(r"e(\d+)", k)
            if m:
                p = os.path.join(f.data_dir, "contents", v.split(":", 1)[1])
                repo[f"f{m.group(1)}"] = {"text": open(p).read(), "isTruncated": False} if os.path.exists(p) else None
        f.out(json.dumps({"data": {"repository": repo}}) + "\n")
    if "reviewThreads" in q:
        f.serve("threads.json", json.dumps(EMPTY_THREADS))
    if "viewer { login }" in q:
        f.serve("review_metadata.json", json.dumps(EMPTY_METADATA))
    if "reviews(first" in q:
        f.serve("review_summaries.json", json.dumps(EMPTY_REVIEWS))
    f.unexpected("unrecognised graphql query")


def rest(f, endpoint):
    m = re.match(r"repos/[^/]+/[^/]+/(.*)", endpoint)
    if not m:
        f.unexpected("unrecognised api endpoint")
    path = m.group(1)
    method = f.flag("--method")
    if method == "POST" and re.fullmatch(r"pulls/\d+/reviews", path):
        f.serve("review_created.json", REVIEW_CREATED)
    if method not in (None, "GET"):
        f.unexpected(f"unexpected {method} to {path}")
    base, _, query = path.partition("?")
    first_page = f.page(query) == "1"
    if re.fullmatch(r"pulls/\d+/files", base):
        f.serve("files.json") if first_page else f.out("[]\n")
    if re.fullmatch(r"pulls/\d+/commits", base):
        f.serve("commits.json") if first_page else f.out("[]\n")
    if base.startswith("compare/"):
        # Range-specific answers: <variant>/compare_end_<end-sha>.{json,patch}; fallback to the PR-wide files.
        end = base[len("compare/"):].split("...")[-1]
        diff_req = any("vnd.github.diff" in a for a in f.argv)
        rel = f"compare_end_{end}.{'patch' if diff_req else 'json'}"
        v = f.variant()
        if v and os.path.exists(os.path.join(f.data_dir, v, rel)):
            f.serve(rel)
        f.serve("pr_diff.patch" if diff_req else "compare.json")
    if base.startswith("contents/"):
        f.serve("contents/" + urllib.parse.unquote(base[len("contents/"):]))
    f.unexpected("unrecognised REST endpoint " + path)
