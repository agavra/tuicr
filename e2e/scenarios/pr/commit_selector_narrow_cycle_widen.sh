#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# PR inline commit selector: Enter narrows to one commit (range diff via compare), ( ) cycle, Space widens.
n=$(e2e_name); sc_pr_open $n
e2e_expect $n '▣   2222222 +third commit'; e2e_expect $n 'second commit'; e2e_expect $n 'first commit'
e2e_keys $n ';k'
e2e_keys $n j Enter          # cursor on bbbbbbb (second commit) -> only it
e2e_wait_for $n 'Commit Message \(bbbbbbb\)'; e2e_wait_for $n 'line from commit B'
e2e_forge_expect "compare/[0-9a-f]+\.\.\.b{40}"
e2e_reject $n 'hello from the PR'
# ) steps to the next older commit (list is newest first), then back to the whole PR
e2e_keys $n ')'; e2e_wait_for $n 'Commit Message \(aaaaaaa\)'; e2e_wait_for $n 'line from commit A'
e2e_forge_expect "compare/[0-9a-f]+\.\.\.a{40}"
e2e_keys $n ')'; e2e_wait_gone $n 'Commit Message \('; e2e_wait_for $n 'new line'; e2e_wait_for $n 'hello from the PR'
# ( goes the other way: from the whole PR to the newest commit
e2e_keys $n '('; e2e_wait_for $n 'Commit Message \(2222222\)'
e2e_forge_expect "compare/[0-9a-f]+\.\.\.2{40}"
e2e_wait_for $n 'A greet\.rs'; e2e_wait_gone $n '▢ M README\.md'
sc_quit $n
