#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Gitea drafts read-back: the PENDING review created by `:submit draft` is read back (its summary body),
# so the post-submit prune does not delete the local review-level comment. Before: the PENDING summary was filtered out.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d
sc_forge_open $n https://gitea.example.com/octo/demo.git
e2e_keys $n ';c'; e2e_wait_for $n 'ctrl-s save'; e2e_keys $n 'gt overall draft'; e2e_settle $n; e2e_keys $n Enter; e2e_wait_for $n 'Review comment added'
sc_cmd $n 'submit draft'; e2e_wait_for $n 'Push pending|Submit review to'; e2e_keys $n y
e2e_wait_for $n 'Pushed pending Gitea review' 15
grep -q 'pulls/1/reviews"\], "stdin": "{.*gt overall draft' $d/forge.log || { echo "no review POST"; cat $d/forge.log; exit 1; }
# the PENDING review is read back and listed in the PR description panel (not pruned/hidden)
e2e_wait_for $n 'gitea @me pending' || { echo "pending review summary not read back"; exit 1; }
e2e_expect $n 'gt overall draft'
sc_quit $n
