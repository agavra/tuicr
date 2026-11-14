# e2e/lib/assert.sh - assertions. Each prints the evidence to stderr and returns non-zero on failure.

# e2e_expect <name> <regex> [timeout]: text appears (render may lag, so this polls).
e2e_expect() { e2e_wait_for "$1" "$2" "${3:-$E2E_TIMEOUT}"; }

# e2e_reject <name> <regex> [window-seconds, may be fractional]: text stays absent for the whole window (default 0.4 s).
# Only meaningful after a positive barrier proved the app reached the state in which the text would show.
e2e_reject() {
  local n=$1 re=$2 t=${3:-0.4} i=0 scr max
  max=$(awk -v t="$t" 'BEGIN { printf "%d", t * 10 + 0.999 }')   # polls every 100 ms; at least one capture for t > 0
  [ "$max" -ge 1 ] || max=1
  while [ $i -lt "$max" ]; do
    scr=$(e2e_screen "$n") || { _e2e_no_pane "$n"; return 2; }
    if grep -Eq -- "$re" <<<"$scr"; then
      echo "e2e: unexpected /$re/ on screen in '$n':" >&2; echo "$scr" >&2; return 1
    fi
    sleep 0.1; i=$((i + 1))
  done
}

# e2e_wait_file <path> [timeout]: a non-empty file appears.
e2e_wait_file() {
  local f=$1 t=${2:-$E2E_TIMEOUT} i=0
  while [ $i -lt $((t * 10)) ]; do [ -s "$f" ] && return 0; sleep 0.1; i=$((i + 1)); done
  echo "e2e: $f did not appear within ${t}s" >&2; return 1
}

# e2e_wait_grep <file> <regex> [timeout]: the file exists and a line matches.
e2e_wait_grep() {
  local f=$1 re=$2 t=${3:-$E2E_TIMEOUT} i=0
  while [ $i -lt $((t * 10)) ]; do grep -Eq -- "$re" "$f" 2>/dev/null && return 0; sleep 0.1; i=$((i + 1)); done
  echo "e2e: no line matching /$re/ in $f within ${t}s" >&2; cat "$f" >&2 2>/dev/null; return 1
}

# e2e_wait_cmd <timeout> <command...>: poll until the command succeeds (state outside the screen: files, git, ...).
e2e_wait_cmd() {
  local t=$1 i=0; shift
  while [ $i -lt $((t * 10)) ]; do "$@" >/dev/null 2>&1 && return 0; sleep 0.1; i=$((i + 1)); done
  echo "e2e: condition never held within ${t}s: $*" >&2; return 1
}

# ---- fake forge call log (one JSON object per line) --------------------------------------------------
e2e_forge_log() { cat "${E2E_FORGE_LOG:?}"; }
e2e_forge_expect() { e2e_forge_log | grep -Eq -- "$1" || { echo "e2e: forge log lacks /$1/" >&2; e2e_forge_log >&2; return 1; }; }
e2e_forge_reject() { ! e2e_forge_log | grep -Eq -- "$1" || { echo "e2e: forge log has /$1/" >&2; e2e_forge_log >&2; return 1; }; }
# e2e_wait_forge_log <regex> [timeout]: a call matching the regex shows up in the log.
e2e_wait_forge_log() {
  local re=$1 t=${2:-$E2E_TIMEOUT} i=0
  while [ $i -lt $((t * 10)) ]; do grep -Eq -- "$re" "${E2E_FORGE_LOG:?}" 2>/dev/null && return 0; sleep 0.1; i=$((i + 1)); done
  echo "e2e: no forge call matching /$re/ within ${t}s" >&2; e2e_forge_log >&2; return 1
}
# e2e_forge_count <regex>: number of logged calls matching.
e2e_forge_count() { grep -Ec -- "$1" "${E2E_FORGE_LOG:?}" 2>/dev/null || true; }
# e2e_wait_forge_count <regex> <min> [timeout]: at least <min> logged calls match (a barrier, not a sleep).
e2e_wait_forge_count() {
  local re=$1 min=$2 t=${3:-$E2E_TIMEOUT} i=0
  while [ $i -lt $((t * 10)) ]; do [ "$(e2e_forge_count "$re")" -ge "$min" ] && return 0; sleep 0.1; i=$((i + 1)); done
  echo "e2e: fewer than $min forge calls matching /$re/ within ${t}s" >&2; e2e_forge_log >&2; return 1
}

# Central check: a call the fakes did not recognise (note "UNEXPECTED") fails the scenario, because tuicr treats
# some forge failures as best-effort and would otherwise hide a wrong request. Scenarios that provoke such
# calls on purpose declare it with `e2e_allow_unexpected_forge_calls <why>`.
e2e_allow_unexpected_forge_calls() { E2E_ALLOW_UNEXPECTED=1; }
e2e_forge_unexpected_ok() {
  local l bad=0
  [ -z "${E2E_FORGE_UNEXPECTED:-}" ] || bad=1
  for l in "${_E2E_FORGE_LOGS[@]:-}"; do
    [ -n "$l" ] && [ -f "$l" ] && grep -q '"note": "UNEXPECTED' "$l" && {
      [ -n "${E2E_ALLOW_UNEXPECTED:-}" ] || { echo "e2e: fake forge saw an unexpected call ($l):" >&2; grep '"note": "UNEXPECTED' "$l" >&2; bad=1; }
    }
  done
  [ $bad -eq 0 ]
}
