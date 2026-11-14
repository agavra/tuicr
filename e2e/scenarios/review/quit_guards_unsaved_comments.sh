#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Adding a comment autosaves it, so a later unsaved change (a reviewed mark) is what `:q` guards while comments exist:
# `:q` refuses, `:w` then lets it through; with no comment at all, reviewed marks alone do not block `:q`.
n=$(e2e_name); sc_diff $n
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'unsaved note'; e2e_keys $n Enter; e2e_wait_for $n 'unsaved note'
sc_cmd $n q; e2e_wait_exit $n   # the comment itself was autosaved: nothing left to lose
n=${n}_dirty; sc_diff $n
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'saved note'; e2e_keys $n Enter; e2e_wait_for $n 'saved note'
e2e_keys $n r; e2e_wait_for $n 'Files · 1/8'      # unsaved change while a comment exists
sc_cmd $n q
e2e_wait_for $n 'No write since last change'
e2e_alive $n || { echo ":q quit with an unsaved comment"; exit 1; }
sc_save $n
sc_cmd $n q
e2e_wait_exit $n
# review-only state (a reviewed mark, no comment) does not block :q
m=${n}_marks; sc_open $m; e2e_wait_for $m 'Files · 0/8'; e2e_keys $m Escape
e2e_keys $m r; e2e_wait_for $m 'Files · 1/8'
sc_cmd $m q; e2e_wait_exit $m
