#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# File-tree filters: i include, e exclude, / path search, I / E clear; filters narrow the diff too.
n=$(e2e_name); sc_diff $n
e2e_keys $n ';h'; e2e_settle $n          # focus the file tree
e2e_keys $n i; e2e_keys $n '^src/'; e2e_keys $n Enter
e2e_wait_for $n 'Files · 0/2'; e2e_wait_gone $n 'README\.md'
e2e_expect $n 'main\.rs'; e2e_expect $n 'lib\.rs'
e2e_keys $n e; e2e_keys $n 'lib'; e2e_keys $n Enter
e2e_wait_for $n 'Files · 0/1'; e2e_wait_gone $n 'lib\.rs'
e2e_keys $n I; e2e_wait_for $n 'Files · 0/7'; e2e_expect $n 'README\.md'; e2e_reject $n '▢ M lib\.rs'
e2e_keys $n E; e2e_wait_for $n 'Files · 0/8'; e2e_expect $n 'lib\.rs'
# invalid regex keeps the prompt open
e2e_keys $n i; e2e_keys $n '(bad'; e2e_keys $n Enter; e2e_wait_for $n '[Ii]nvalid|[Rr]egex|error'
e2e_keys $n Escape
# path search moves the tree selection
e2e_keys $n '/'; e2e_keys $n 'tabs'; e2e_keys $n Enter
e2e_keys $n Enter; e2e_wait_for $n '┌ tabs\.txt ─'
sc_quit $n
