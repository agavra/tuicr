#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# PR open through fake gh: description panel (status, reviewers, checks), r collapses/expands it.
n=$(e2e_name); sc_pr_open $n
e2e_expect $n 'PR #1 Add greeting and tweak README'; e2e_expect $n 'More description text here'
e2e_expect $n 'Status: OPEN · REVIEW REQUIRED'; e2e_expect $n 'Requested: @bob'; e2e_expect $n 'Approved: @carol'
e2e_expect $n '✓ build'; e2e_expect $n '✗ lint'
e2e_expect $n 'PR Mode · octo/demo#1'
e2e_keys $n r
e2e_wait_for $n '═══ ✓ PR #1'; e2e_wait_gone $n 'More description text here'
e2e_keys $n r
e2e_wait_gone $n '═══ ✓ PR #1'; e2e_wait_for $n 'More description text here'
# marking description reviewed persists (:w)
e2e_keys $n r; e2e_wait_for $n '═══ ✓ PR #1'
sc_save $n
sc_session_json $n | tr -d ' \n' | grep -q '"pr_description_reviewed":true' || { echo "pr_description_reviewed not saved"; exit 1; }
sc_quit $n
