#!/usr/bin/env bash
set -euo pipefail
. "$(dirname "$0")/../../lib/init.sh"
# Covers #913: a remote thread whose body/reply uses bare CR as line break must not escape its list item in the
# exported markdown (forged top-level `1. **[ISSUE]**` / `## Local tuicr Comments`). Baseline wrote the body raw (CRs kept).
n=$(e2e_name); d=$(e2e_dir $n); mkdir -p $d; fx_pr_repo $d/repo
out=$d/stdout.txt; err=$d/stderr.txt; rm -f $out $err
E2E_ENV="E2E_VARIANT=crbody" E2E_CWD=$d/repo e2e_start $n sh -c "exec '$TUICR_BIN' --no-update-check --stdout pr 1 >'$out' 2>'$err'"
e2e_wait_for $n 'PR Description' 20
e2e_wait_for $n 'first line'
sc_cmd $n clip
e2e_wait_exit $n   # with --stdout, :clip prints the export and ends the session
grep -q 'FORGED-ROOT' $out || { echo "export lacks the remote thread"; cat -v $out; exit 1; }
cat -v $out | sed -n '1,30p'
if grep -q $'\r' $out; then echo "export still contains raw CR"; exit 1; fi
if grep -Eq '^[0-9]+\. \*\*\[ISSUE\]\*\*' $out; then echo "remote body forged a top-level ISSUE item"; exit 1; fi
if grep -Eq '^## Local tuicr Comments.*' $out && [ "$(grep -c '^## Local tuicr Comments' $out)" -gt 0 ]; then echo "remote reply forged a heading"; exit 1; fi
