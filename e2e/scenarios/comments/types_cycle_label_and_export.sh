#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Configured `comment_types`: Tab / Shift-Tab cycle them (config order, then none) showing the config *label*; the export
# tags the comment with that label, leaves an untyped comment bare, and lists the type used, with its definition, in the legend.
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d/config/tuicr; fx_git_repo $d/repo
cat > $d/config/tuicr/config.toml <<'T'
comment_types = [
  { id = "note", label = "question", definition = "ask for clarification" },
  { id = "issue", definition = "problems to fix" },
  { id = "nit", label = "nitpick", definition = "small tweaks" },
]
T
out=$d/stdout.txt; rm -f $out
E2E_CWD=$d/repo e2e_start_keep $n sh -c "exec '$TUICR_BIN' --no-update-check --stdout -r HEAD~2..HEAD >'$out' 2>/dev/null"
e2e_wait_for $n 'Files · 0/8'; e2e_keys $n Escape
sc_cursor_on $n '4 ▌ second line'
e2e_keys $n c; e2e_wait_for $n 'Add \[QUESTION\]'                       # first configured type is the default
e2e_keys $n Tab; e2e_wait_for $n 'Add \[ISSUE\]'                        # no label: the id
e2e_keys $n Tab; e2e_wait_for $n 'Add \[NITPICK\]'
e2e_keys $n Tab; e2e_wait_for $n 'Comment type: none'                   # none closes the cycle
e2e_keys $n Tab; e2e_wait_for $n 'Add \[QUESTION\]'
e2e_keys $n BTab; e2e_wait_for $n 'Comment type: none'                  # and Shift-Tab walks it backwards
e2e_keys $n BTab; e2e_wait_for $n 'Add \[NITPICK\]'
e2e_keys $n 'tiny detail'; e2e_keys $n Enter; e2e_wait_for $n 'tiny detail'
e2e_expect $n '\[NITPICK\]'                                            # the saved comment shows its label
e2e_keys $n g; sc_cursor_on $n '3   first line'
e2e_keys $n c; e2e_wait_for $n 'Add \[QUESTION\]'; e2e_keys $n Tab Tab Tab; e2e_wait_for $n 'Comment type: none'
e2e_keys $n 'plain remark'; e2e_keys $n Enter; e2e_wait_for $n 'plain remark'
sc_cmd $n clip; e2e_wait_exit $n
grep -Eq '\*\*\[NITPICK\]\*\*.*README\.md:4.*tiny detail' $out || { echo "export lacks the labelled comment:"; cat $out; exit 1; }
grep -E 'plain remark' $out | grep -q '\[' && { echo "untyped comment got a tag:"; grep 'plain remark' $out; exit 1; }
grep -q 'plain remark' $out || { echo "export lacks the untyped comment:"; cat $out; exit 1; }
grep -q 'Comment types: NITPICK (small tweaks)' $out || { echo "legend lacks the used type and its definition:"; cat $out; exit 1; }
