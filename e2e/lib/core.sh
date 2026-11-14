# e2e/lib/core.sh - run environment, isolation, cleanup, failure bookkeeping.

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${TUICR_BIN:?set TUICR_BIN to the tuicr binary}"
# run.sh exports a unique E2E_ROOT/E2E_SOCKET; a scenario run by hand gets its own.
E2E_ROOT="${E2E_ROOT:-$(mktemp -d "${TMPDIR:-/tmp}/tuicr-e2e.XXXXXX")}"
E2E_SOCKET="${E2E_SOCKET:-tuicr-e2e-$$}"
E2E_OUT="${E2E_OUT:-$E2E_ROOT/out}"
E2E_COLS="${E2E_COLS:-160}"
E2E_ROWS="${E2E_ROWS:-45}"
E2E_TIMEOUT="${E2E_TIMEOUT:-10}"
mkdir -p "$E2E_OUT"
export E2E_DIR E2E_ROOT E2E_SOCKET E2E_OUT

# Scenario name = <area>_<behaviour>, from the script path (scenarios/<area>/<behaviour>.sh).
e2e_name() { echo "$(basename "$(dirname "$E2E_SCENARIO")")_$(basename "$E2E_SCENARIO" .sh)"; }
e2e_dir() { echo "$E2E_ROOT/$1"; }   # per-scenario scratch dir

# Harness-side git/hg/locale isolation: the developer's or CI runner's global config must not leak into fixtures.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0
export GIT_AUTHOR_NAME=E2E GIT_AUTHOR_EMAIL=e2e@example.invalid GIT_COMMITTER_NAME=E2E GIT_COMMITTER_EMAIL=e2e@example.invalid
export HGRCPATH=/dev/null HGPLAIN=1 HGUSER=e2e

# UTF-8 locale for tuicr and tmux: en_US.UTF-8 where installed, else C.UTF-8 (CI images).
if [ -z "${E2E_LOCALE:-}" ]; then
  if locale -a 2>/dev/null | grep -qix 'en_US.utf-\?8'; then E2E_LOCALE=en_US.UTF-8; else E2E_LOCALE=C.UTF-8; fi
fi

export LANG="$E2E_LOCALE" LC_ALL="$E2E_LOCALE"

# PATH of the child: fake forge CLIs first, then system dirs, then the dirs of the tools the harness needs.
_e2e_tool_path=/usr/bin:/bin
for _t in git python3; do
  _p=$(command -v "$_t") || { echo "e2e: required tool '$_t' not found" >&2; exit 2; }
  _e2e_tool_path="$_e2e_tool_path:$(dirname "$_p")"
done
E2E_TOOL_PATH="$_e2e_tool_path${E2E_EXTRA_PATH:+:$E2E_EXTRA_PATH}"
unset _t _p _e2e_tool_path

# e2e_skip <reason>: end the scenario as SKIPPED (exit 77, the runner reports it separately).
e2e_skip() { echo "SKIP: $*"; trap - EXIT; exit 77; }

# e2e_on_exit <command...>: register cleanup, run in reverse order on exit, error or signal.
_E2E_CLEANUPS=()
e2e_on_exit() { _E2E_CLEANUPS+=("$*"); }
_e2e_exit() {
  local rc=$? i
  trap - EXIT
  e2e_stop_all
  for ((i = ${#_E2E_CLEANUPS[@]} - 1; i >= 0; i--)); do eval "${_E2E_CLEANUPS[i]}" || true; done
  # No scenario may pass while a fake forge saw a call its fixtures do not cover.
  [ $rc -ne 0 ] || e2e_forge_unexpected_ok || rc=1
  exit $rc
}
trap _e2e_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
