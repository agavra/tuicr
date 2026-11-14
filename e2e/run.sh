#!/usr/bin/env bash
# e2e/run.sh - run the end-to-end scenarios against a real tuicr binary.
#
#   TUICR_BIN=target/release/tuicr e2e/run.sh [-l|--list] [filter...]
#
# A filter is a substring of `<area>/<behaviour>` (e.g. `pr/submit`, `tree/`, `theme/picker_preview_revert_keep`);
# no filter runs everything. Results: PASS, FAIL (includes timeouts), SKIP (scenario exit 77, e.g. no Mercurial),
# and QPASS/QFAIL for quarantined scenarios (e2e/quarantine.tsv: run and reported, never gating).
# Exit 0 only if at least one scenario ran, none FAILed, and (with E2E_REQUIRE_SKIPS_ZERO=1) none was skipped.
#
# Environment: TUICR_BIN (required); E2E_ROOT (scratch, default a fresh mktemp dir), E2E_OUT (logs, screenshots,
# results.tsv; default $E2E_ROOT/out), E2E_SOCKET (tmux socket, default unique), E2E_SCENARIO_TIMEOUT (s, default 120),
# E2E_SCENARIO_DIR (default e2e/scenarios; a scenario is <area>/<behaviour>.sh or .py, never both), E2E_KEEP_ROOT=1 (keep scratch dirs of passing scenarios), E2E_REQUIRE_HG=1.
set -u
here="$(cd "$(dirname "$0")" && pwd)"

list_only=
filters=()
for a in "$@"; do case $a in -l|--list) list_only=1;; *) filters+=("$a");; esac; done

scen_dir=${E2E_SCENARIO_DIR:-$here/scenarios}
selected=()
while IFS= read -r s; do
  name=${s#"$scen_dir"/}; name=${name%.sh}; name=${name%.py}
  if [ ${#filters[@]} -gt 0 ]; then
    ok=; for f in "${filters[@]}"; do [[ "$name" == *"$f"* ]] && ok=1; done
    [ -n "$ok" ] || continue
  fi
  selected+=("$name")
done < <(find "$scen_dir" \( -name '*.sh' -o -name '*.py' \) | sort)
# a behaviour lives in exactly one language: <area>/<behaviour>.sh and .py side by side is an error
dups=$(printf '%s\n' "${selected[@]:-}" | sort | uniq -d)
[ -z "$dups" ] || { echo "e2e: scenario in both bash and python: $dups" >&2; exit 2; }

if [ ${#selected[@]} -eq 0 ]; then echo "e2e: no scenario matches '${filters[*]:-}' under $scen_dir" >&2; exit 2; fi
if [ -n "$list_only" ]; then printf '%s\n' "${selected[@]}"; exit 0; fi

: "${TUICR_BIN:?set TUICR_BIN to the tuicr binary}"
[ -x "$TUICR_BIN" ] || { echo "e2e: TUICR_BIN=$TUICR_BIN is not an executable file" >&2; exit 2; }
export TUICR_BIN; TUICR_BIN="$(cd "$(dirname "$TUICR_BIN")" && pwd)/$(basename "$TUICR_BIN")"
export PYTHONPATH="$here/lib${PYTHONPATH:+:$PYTHONPATH}"   # python scenarios: `from e2e import scenario`
own_root=
if [ -z "${E2E_ROOT:-}" ]; then E2E_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/tuicr-e2e.XXXXXX"); own_root=1; fi
export E2E_ROOT E2E_SOCKET="${E2E_SOCKET:-tuicr-e2e-$$}"
export E2E_OUT="${E2E_OUT:-$E2E_ROOT/out}"
timeout_s=${E2E_SCENARIO_TIMEOUT:-120}
mkdir -p "$E2E_OUT"; : > "$E2E_OUT/results.tsv"

quarantined() { awk -F'\t' -v n="$1" '$1 == n { print $2; found = 1 } END { exit !found }' "${E2E_QUARANTINE_FILE:-$here/quarantine.tsv}" 2>/dev/null; }

cleanup() {
  tmux -L "$E2E_SOCKET" kill-server 2>/dev/null
  [ -z "$own_root" ] || [ -n "${E2E_KEEP_ROOT:-}" ] || [ "$fail" -ne 0 ] || find "$E2E_ROOT" -mindepth 1 -maxdepth 1 ! -name out -exec rm -rf {} + 2>/dev/null
}
on_signal() { echo; echo "e2e: interrupted"; cleanup; exit 130; }
trap on_signal INT TERM

pass=0; fail=0; skip=0; qpass=0; qfail=0
for name in "${selected[@]}"; do
  mkdir -p "$E2E_OUT/$(dirname "$name")"
  log="$E2E_OUT/$name.log"; start=$(date +%s)
  if [ -f "$scen_dir/$name.py" ]; then script=$scen_dir/$name.py; interp=python3; else script=$scen_dir/$name.sh; interp=bash; fi
  E2E_SCENARIO="$script" python3 "$here/lib/run_scenario.py" "$timeout_s" "$interp" "$script" >"$log" 2>&1
  rc=$?; secs=$(( $(date +%s) - start ))
  if reason=$(quarantined "$name"); then
    if [ $rc -eq 0 ]; then res=QPASS; qpass=$((qpass+1)); else res=QFAIL; qfail=$((qfail+1)); fi
    echo "$res $name (${secs}s)  quarantined: $reason"
  elif [ $rc -eq 0 ]; then res=PASS; pass=$((pass+1)); echo "PASS $name (${secs}s)"
  elif [ $rc -eq 77 ]; then res=SKIP; skip=$((skip+1)); echo "SKIP $name  $(grep -m1 '^SKIP' "$log")"
  else
    res=FAIL; fail=$((fail+1))
    echo "FAIL $name (${secs}s, rc=$rc)  log: $log"
  fi
  printf '%s\t%s\t%s\n' "$name" "$res" "$secs" >> "$E2E_OUT/results.tsv"
done

cleanup
echo "== ${#selected[@]} scenarios: $pass PASS, $fail FAIL, $skip SKIP, $qpass QPASS, $qfail QFAIL  (results: $E2E_OUT/results.tsv)"
[ "$fail" -eq 0 ] || exit 1
[ -z "${E2E_REQUIRE_SKIPS_ZERO:-}" ] || [ "$skip" -eq 0 ] || { echo "e2e: $skip scenario(s) skipped but E2E_REQUIRE_SKIPS_ZERO=1" >&2; exit 1; }
exit 0
