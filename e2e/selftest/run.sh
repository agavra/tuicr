#!/usr/bin/env bash
# e2e/selftest/run.sh - tests of the harness itself: every way the runner could report a false green must be
# caught here. Needs only tmux, python3 and git (TUICR_BIN is never started; it defaults to `env`).
#   e2e/selftest/run.sh
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root=$(mktemp -d "${TMPDIR:-/tmp}/tuicr-e2e-selftest.XXXXXX")
fail=0
trap 'tmux -L "st-$$" kill-server 2>/dev/null; rm -rf "$root"' EXIT

check() { if [ "$2" = "$3" ]; then echo "ok   $1 ($3)"; else echo "FAIL $1: expected $2, got $3"; fail=1; fi; }

export TUICR_BIN=${TUICR_BIN:-$(command -v env)} E2E_SOCKET="st-$$" E2E_ROOT="$root" E2E_OUT="$root/out"
E2E_SCENARIO_DIR="$here/scenarios" E2E_QUARANTINE_FILE="$here/quarantine.tsv" E2E_SCENARIO_TIMEOUT=${SELFTEST_TIMEOUT:-60} \
  "$here/../run.sh" > "$root/run.txt" 2>&1
rc=$?
# The hung scenario runs on its own with a short budget, so the PASS scenarios above never race a tight timeout.
E2E_ROOT="$root/h" E2E_OUT="$root/h/out" E2E_SCENARIO_DIR="$here/scenarios-hung" E2E_QUARANTINE_FILE="$here/quarantine.tsv" E2E_SCENARIO_TIMEOUT=4 \
  "$here/../run.sh" >> "$root/run.txt" 2>&1
hrc=$?
cat "$root/h/out/results.tsv" >> "$root/out/results.tsv"
cat "$root/run.txt"
echo "-- runner exit status with failing scenarios present"
check "run.sh exits non-zero when scenarios FAIL" 1 "$rc"
check "run.sh exits non-zero when a scenario hangs" 1 "$hrc"

while IFS=$'\t' read -r name want; do
  got=$(awk -F'\t' -v n="$name" '$1 == n { print $2 }' "$root/out/results.tsv")
  check "$name" "$want" "${got:-missing}"
done < "$here/expect.tsv"

echo "-- hung scenario's background child must be dead"
pid=$(cat "$root/hung_child.pid" 2>/dev/null)
if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then echo "FAIL child $pid still running"; kill -9 "$pid"; fail=1; else echo "ok   child process tree was killed"; fi
grep -q 'rc=124' "$root/run.txt" && echo "ok   timeout reported as rc=124" || { echo "FAIL timeout not reported as rc=124"; fail=1; }

echo "-- a filter that selects nothing must fail"
E2E_SCENARIO_DIR="$here/scenarios" "$here/../run.sh" no/such/scenario >/dev/null 2>&1
check "zero selected scenarios exits 2" 2 "$?"

echo "-- all-green run exits 0; E2E_REQUIRE_SKIPS_ZERO turns a skip into a failure"
E2E_ROOT="$root/g" E2E_OUT="$root/g/out" E2E_SCENARIO_DIR="$here/scenarios" E2E_QUARANTINE_FILE="$here/quarantine.tsv" "$here/../run.sh" missing_pane exit_status >/dev/null 2>&1
check "passing selection exits 0" 0 "$?"
E2E_ROOT="$root/s" E2E_OUT="$root/s/out" E2E_SCENARIO_DIR="$here/scenarios" E2E_QUARANTINE_FILE="$here/quarantine.tsv" "$here/../run.sh" skip_is_reported >/dev/null 2>&1
check "a skip alone exits 0" 0 "$?"
E2E_REQUIRE_SKIPS_ZERO=1 E2E_ROOT="$root/s2" E2E_OUT="$root/s2/out" E2E_SCENARIO_DIR="$here/scenarios" E2E_QUARANTINE_FILE="$here/quarantine.tsv" "$here/../run.sh" skip_is_reported >/dev/null 2>&1
check "a skip with E2E_REQUIRE_SKIPS_ZERO=1 exits 1" 1 "$?"

echo "-- tracked e2e/ file names must be valid on Windows"
bad=$(git -C "$here/.." ls-files -z -- . | python3 -c '
import re, sys
for p in sys.stdin.read().split("\0"):
    if any(re.search(r"[<>:\"\\|?*\x00-\x1f]|[. ]$", part) for part in p.split("/")):
        print(p)')
if [ -z "$bad" ]; then echo "ok   no tracked e2e/ name has Windows-invalid characters"; else echo "FAIL Windows-invalid file names:"; echo "$bad"; fail=1; fi

[ $fail -eq 0 ] && echo "selftest: all checks passed" || echo "selftest: FAILED"
exit $fail
