#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Add line / range / file / review comments; they render inline and persist with :w.
n=$(e2e_name); sc_diff $n
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n 'COMMENT'
e2e_keys $n 'line note alpha'; e2e_keys $n Enter
e2e_wait_for $n 'line note alpha'; e2e_wait_for $n 'NORMAL'; e2e_expect $n '── L4 '   # composing ended before the rendered box is checked
# range comment over two rows (gone.txt deleted lines) via visual mode
sc_cursor_on $n '1 ▌ delete me'
e2e_keys $n v j c; e2e_wait_for $n 'COMMENT'
e2e_keys $n 'range note beta'; e2e_keys $n Enter
e2e_wait_for $n 'range note beta'; e2e_wait_for $n 'NORMAL'; e2e_expect $n 'L1-L2|L1.{0,3}2'
# file comment
e2e_keys $n g; e2e_keys $n C; e2e_wait_for $n 'COMMENT'
e2e_keys $n 'file note gamma'; e2e_keys $n Enter
e2e_wait_for $n 'file note gamma'; e2e_wait_for $n 'NORMAL'
# review-level comment
e2e_keys $n ';c'; e2e_wait_for $n 'COMMENT'
e2e_keys $n 'review note delta'; e2e_keys $n Enter
e2e_wait_for $n 'review note delta'; e2e_wait_for $n 'NORMAL'
# comment navigator sidebar lists them
e2e_expect $n 'Comments'
# persist, then verify on disk
sc_save $n
# each comment is stored under its own anchor, not just somewhere in the file
sc_session_json $n | python3 -c '
import json, sys
j = json.load(sys.stdin)
f = j["files"]
def one(cs, text):
    assert len(cs) == 1 and cs[0]["content"] == text, (text, cs)
    return cs[0]
l = one(f["README.md"]["line_comments"]["4"], "line note alpha")
assert l["side"] == "new" and l["line_range"] is None, l
r = one(f["gone.txt"]["line_comments"]["2"], "range note beta")
assert r["side"] == "old" and r["line_range"] == {"start": 1, "end": 2}, r      # deleted lines: old side, L1-L2
one(f["README.md"]["file_comments"], "file note gamma")
one(j["review_comments"], "review note delta")
# nothing leaked into another bucket
assert sorted(c["content"] for fr in f.values() for cs in fr["line_comments"].values() for c in cs) == ["line note alpha", "range note beta"]
assert sum(len(fr["file_comments"]) for fr in f.values()) == 1 and len(j["review_comments"]) == 1
' || { echo "session json has the comments in the wrong places:"; sc_session_json $n | head -c 3000; exit 1; }
sc_quit $n
