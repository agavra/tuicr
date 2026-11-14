#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #892: a file-level inline comment (inline.path, no from/to) and a live reply to a deleted root
# comment are kept. Baseline: the file-level thread and the reply (with its deleted root) are dropped.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d
cat > $d/comments.json <<'JSON'
[{"id":1,"content":{"raw":"BB-FILE-LEVEL-NOTE"},"user":{"display_name":"carol"},"parent":null,"inline":{"path":"src/greet.rs","from":null,"to":null}},
 {"id":2,"content":{"raw":"gone"},"user":{"display_name":"dave"},"parent":null,"deleted":true,"inline":{"path":"src/greet.rs","from":null,"to":1}},
 {"id":3,"content":{"raw":"BB-REPLY-TO-DELETED"},"user":{"display_name":"erin"},"parent":{"id":2},"inline":{"path":"src/greet.rs","from":null,"to":1}}]
JSON
E2E_ENV="E2E_BB_COMMENTS=$d/comments.json" sc_forge_open $n https://bitbucket.org/octo/demo.git
# the file-level note renders in the PR description pane (top); check before `}` scrolls it away
e2e_wait_for $n 'BB-FILE-LEVEL-NOTE' 10 || { echo "file-level comment dropped"; e2e_screen $n; exit 1; }
e2e_keys $n '}'
e2e_wait_for $n 'BB-REPLY-TO-DELETED' 8 || { echo "reply to a deleted root dropped"; e2e_screen $n; exit 1; }
e2e_screen $n | grep -q '\[deleted\]' || { echo "no [deleted] placeholder root"; e2e_screen $n; exit 1; }
sc_quit $n
