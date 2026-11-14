#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# after `:submit draft` on GitLab the pushed drafts (inline + review body) are read back from
# /draft_notes. Before the fix the refetch found none and prune_locked_comments deleted the only local copy.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d
sc_forge_open $n https://gitlab.com/octo/demo.git
e2e_keys $n '}'; sc_cursor_on $n '1[╭ ]▌ pub fn greet'
e2e_keys $n c; e2e_wait_for $n COMMENT; e2e_keys $n 'gl draft inline'; e2e_keys $n Enter; e2e_wait_for $n 'gl draft inline'
sc_cmd $n 'submit draft'; e2e_wait_for $n 'Inline: 1'; e2e_keys $n y
e2e_wait_for $n 'Pushed|pushed|Submitted' 15
# the draft note really reached the forge
grep -q 'merge_requests/1/draft_notes", "--method", "POST"' $d/forge.log && grep -q 'gl draft inline' $d/forge.log || { echo "no draft POST"; cat $d/forge.log; exit 1; }
# ...and the refetch shows it back as a remote draft thread (navigator row "R src/greet.rs:1"), not pruned
e2e_wait_for $n 'R src/greet.rs:1' || { echo "pushed draft was pruned: not read back from /draft_notes"; exit 1; }
sc_quit $n
