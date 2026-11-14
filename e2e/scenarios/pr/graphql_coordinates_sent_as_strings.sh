#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #915: GraphQL repository coordinates must be sent with `gh api -f` (string), not `-F` (typed: a repo named
# `123` becomes the integer 123 and GitHub rejects the query). Repo octo/123; assert no `-F owner=/name=/after=` in the log.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_pr_repo $d/repo
_fx_git $d/repo remote set-url origin https://github.com/octo/123.git
E2E_ENV="E2E_VARIANT=threads_and_commits" E2E_CWD=$d/repo e2e_start $n "$TUICR_BIN" --no-update-check pr 1
e2e_wait_for $n 'PR Description' 20; e2e_wait_for $n 'remote open thread'
python3 - <<PY
import json
for l in open("$E2E_FORGE_LOG"):
    a = json.loads(l)["argv"]
    if "graphql" in a:
        print(" ".join(x for x in a if not x.startswith("query="))[:150])
PY
if python3 - <<PY
import json, sys
bad = 0
for l in open("$E2E_FORGE_LOG"):
    a = json.loads(l)["argv"]
    if "graphql" in a:
        for i, x in enumerate(a):
            if x == "-F" and a[i + 1].split("=", 1)[0] in ("owner", "name", "after"): bad += 1
sys.exit(1 if bad else 0)
PY
then :; else echo "GraphQL owner/name/after sent with typed -F"; exit 1; fi
sc_quit $n
