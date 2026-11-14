#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# `tuicr review` on a saved session without a TUI: JSON payloads (stdin and literal), flag comments, `delete` by id,
# `clearc` (keeps review marks) and `clear` (resets them, then removes the emptied session file).
n=$(e2e_name); sc_diff $n
repo="$(e2e_dir $n)/repo"
e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'; sc_save $n; sc_quit $n     # a session with one reviewed file
slug=$(sc_run $n review list --repo "$repo" | python3 -c 'import json,sys; l=json.load(sys.stdin); assert len(l)==1, l; print(l[0]["slug"])')
sess() { sc_run $n review "$1" --repo "$repo" --session "$slug" "${@:2}"; }
listed() { local o; o=$(sess comments); grep -q -- "$1" <<<"$o"; }   # captured first: no SIGPIPE games under pipefail
id_of() { python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])'; }
sess add --input - >/dev/null <<'JSON'
{"type": "issue", "content": "from stdin json", "file": "README.md", "line": 4, "side": "new"}
JSON
sess add --input '{"comment_type": "suggestion", "content": "range from literal json", "target": {"type": "line_range", "file": "README.md", "start_line": 3, "end_line": 4, "side": "new"}}' >/dev/null
id=$(sess add --target-file src/lib.rs --username alice "flag file note" | id_of)
out=$(sess comments)
python3 - "$out" <<'PY'
import json, sys
c = json.loads(sys.argv[1]); c = c if isinstance(c, list) else c.get("comments", c)
by = {x["content"]: x for x in c}
assert set(by) == {"from stdin json", "range from literal json", "flag file note"}, sorted(by)
x = by["from stdin json"]; assert (x["location"], x["comment_type"], x["side"]) == ("README.md:4", "issue", "new"), x
x = by["range from literal json"]; assert (x["start_line"], x["end_line"], x["comment_type"]) == (3, 4, "suggestion"), x
x = by["flag file note"]; assert x["path"] == "src/lib.rs" and "start_line" not in x or x["start_line"] is None, x
assert by["flag file note"]["author"] == "alice", by["flag file note"]
PY
sess delete --comment-id "$id" | grep -q "$id"
listed 'flag file note' && { echo "deleted comment still listed"; exit 1; }
sess delete --comment-id "$id" 2>/dev/null && { echo "deleting a missing id succeeded"; exit 1; }
sess clearc >/dev/null
listed 'from stdin json' && { echo "clearc left comments"; exit 1; }
[ -n "$(sc_session_json $n)" ] || { echo "clearc removed a session that still has a reviewed file"; exit 1; }
sess clear >/dev/null
[ -z "$(sc_session_json $n)" ] || { echo "clear left an empty session behind"; sc_session_json $n; exit 1; }
