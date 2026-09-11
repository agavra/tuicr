#!/usr/bin/env bash
set -e -u -o pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_tuicr-common.sh"

# Configuration - override via environment variables
TUICR_PANE_DIRECTION="${TUICR_PANE_DIRECTION:-stacked}"  # down or right or stacked
ZELLIJ_BIN="${ZELLIJ_BIN:-zellij}"

# Backward-compat: map old tmux-style position to zellij direction
case "${TUICR_PANE_POSITION:-}" in
  top|bottom) TUICR_PANE_DIRECTION="down" ;;
  left|right) TUICR_PANE_DIRECTION="right" ;;
  stacked) TUICR_PANE_DIRECTION="stacked" ;;
esac

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
  echo -e "${GREEN}[tuicr]${NC} $*"
}

log_warn() {
  echo -e "${YELLOW}[tuicr]${NC} $*"
}

log_error() {
  echo -e "${RED}[tuicr]${NC} $*"
}

usage() {
  cat << EOF
Usage: $(basename "$0") [directory] [-- tuicr-args...]

Launch tuicr in a zellij split pane to review changes.

Arguments:
  directory    Git or jj repository directory to review (default: current directory)
  tuicr-args   Extra arguments passed through to tuicr (e.g. -w, -r <revset>)

Environment variables:
  TUICR_PANE_DIRECTION  Split direction: down or right or stacked (default: stacked)
  ZELLIJ_BIN            Path to zellij executable

Examples:
  $(basename "$0")                            # Review changes in current directory
  $(basename "$0") ~/project                  # Review changes in ~/project
  $(basename "$0") . -- -w                    # Review uncommitted working-tree changes
  TUICR_PANE_DIRECTION=right $(basename "$0") # Split to the right instead of below
EOF
}

check_zellij() {
  if [[ ! -x "$ZELLIJ_BIN" ]] && ! command -v zellij &> /dev/null; then
    log_error "zellij command not found on PATH"
    return 1
  fi
  return 0
}

check_tuicr() {
  if ! command -v tuicr &> /dev/null; then
    log_error "tuicr not found. Install it first."
    return 1
  fi
  return 0
}

check_repo() {
  local dir="$1"
  if git -C "$dir" rev-parse --git-dir &> /dev/null; then
    return 0
  fi
  if command -v jj &> /dev/null \
    && jj --repository "$dir" --ignore-working-copy root &> /dev/null; then
    return 0
  fi
  log_error "Not a git or jj repository: $dir"
  return 1
}

# Whether the pane this wrapper was called from is a zellij floating overlay.
#
# zellij's layout dump groups floating panes under `floating_panes {` and
# carries no pane ids, so the caller has to be recognised by its command line.
# That is not this shell — an agent calls the wrapper several processes deep —
# so walk up to the pane's own process and try each ancestor.
caller_pane_is_floating() {
  local layout pid cmd
  layout=$("$ZELLIJ_BIN" action dump-layout 2>/dev/null) || return 1

  pid=$$
  while [[ -n "$pid" && "$pid" != "1" ]]; do
    cmd=$(tr '\0' '\n' < "/proc/$pid/cmdline" 2>/dev/null | head -1)
    if [[ -n "$cmd" && "$cmd" != "bash" && "$cmd" != "sh" ]]; then
      if printf '%s\n' "$layout" | awk -v needle="$cmd" '
           # Only the focused tab: a floating pane in some other tab says
           # nothing about where this one is.
           /^    tab / { in_tab = ($0 ~ /focus=true/); inside = 0 }
           in_tab && /floating_panes \{/ { inside = 1 }
           inside && index($0, needle) { found = 1 }
           inside && /^        \}/ { inside = 0 }
           END { exit(found ? 0 : 1) }
         '; then
        return 0
      fi
    fi
    pid=$(awk '{print $4}' "/proc/$pid/stat" 2>/dev/null)
  done
  return 1
}

check_tuicr_running() {
  # Zellij has no panes-list CLI like tmux. Fall back to a process check —
  # scoped to the repository being opened, since a tuicr reviewing some other
  # checkout is no reason to refuse this one.
  local target="$1"
  local pid cwd
  for pid in $(pgrep -x tuicr 2>/dev/null); do
    cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null) || continue
    if [[ "$cwd" == "$target" || "$cwd" == "$target"/* ]]; then
      return 0
    fi
  done
  return 1
}

launch_tuicr_pane() {
  local target_dir="$1"
  shift
  local tuicr_args=("$@")

  # Validate direction
  case "$TUICR_PANE_DIRECTION" in
    down|right|stacked) ;;
    *)
      log_warn "Unknown TUICR_PANE_DIRECTION '$TUICR_PANE_DIRECTION', defaulting to 'stacked'"
      TUICR_PANE_DIRECTION="stacked"
      ;;
  esac

  log_info "Launching tuicr in $TUICR_PANE_DIRECTION-split pane"
  log_info "Directory: $target_dir"

  # FIFO for blocking until tuicr exits (zellij has no wait-for primitive)
  local fifo
  fifo=$(mktemp -u "/tmp/tuicr-fifo.XXXXXX")
  mkfifo "$fifo"

  # Optional --stdout capture
  local output_file=""
  local tuicr_cmd="$(command -v tuicr)$(tuicr_quote_args "${tuicr_args[@]+"${tuicr_args[@]}"}")"
  local use_stdout=false

  if tuicr_stdout_supported; then
    output_file=$(mktemp /tmp/tuicr-output.XXXXXX)
    tuicr_cmd="$tuicr_cmd --stdout > '$output_file'"
    use_stdout=true
    log_info "Using --stdout mode (output will be captured)"
  else
    log_warn "tuicr --stdout not supported, output will be copied to clipboard"
  fi

  # Spawn tuicr in a new zellij pane. --close-on-exit cleans up the pane when
  # tuicr quits; the trailing FIFO write unblocks the wrapper.
  # Without --near-current-pane zellij opens beside whatever is FOCUSED, which
  # is the tab the human is looking at rather than the pane this wrapper runs
  # in — an agent's review lands in someone else's tab.
  zellij_args=("--close-on-exit" "--near-current-pane" "--name" "tuicr")

  # A stack is only visible from a tiled pane. Called from a floating overlay,
  # a stacked pane is hidden behind whatever is already stacked there, so the
  # review appears not to have opened at all — split instead.
  local direction="$TUICR_PANE_DIRECTION"
  if [[ "$direction" == "stacked" ]] && caller_pane_is_floating; then
    direction="right"
    log_info "Caller is a floating pane; opening a split instead of a stack"
  fi

  if [[ "$direction" == "stacked" ]]; then
    zellij_args+=("--stacked")
  else
    zellij_args+=("--direction" "$direction")
  fi

  # bash, not sh: $tuicr_cmd carries bash's printf %q quoting, which can emit
  # non-POSIX $'...' forms that dash (the default /bin/sh on many Linux
  # distros) does not understand.
  zellij_args+=(-- bash -c "$tuicr_cmd; echo done > '$fifo'")

  "$ZELLIJ_BIN" run\
    "${zellij_args[@]}"


  log_info "tuicr is running in a $direction pane"
  log_info "Waiting for tuicr to exit..."

  # Block until the spawned command writes to the FIFO
  read -r _ < "$fifo"
  rm -f "$fifo"

  log_info "tuicr finished"

  tuicr_report_stdout_output "$use_stdout" "$output_file"
}

main() {
  # Handle help
  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
  fi

  # Check for tuicr
  if ! check_tuicr; then
    exit 1
  fi

  # Determine target directory, then split off any pass-through tuicr args
  tuicr_parse_args "$@"
  local target_dir="$TUICR_TARGET_DIR"
  target_dir=$(cd "$target_dir" && pwd)  # Get absolute path

  # Verify it's a git or jj repo
  if ! check_repo "$target_dir"; then
    exit 1
  fi

  # Check if we're in zellij
  if ! check_zellij; then
    log_error "Not running inside zellij!"
    echo ""
    echo "To use tuicr with your coding agent, run that agent inside zellij."
    echo ""
    echo "1. Exit the current agent session."
    echo ""
    echo "2. Restart the agent inside zellij (e.g. 'zellij' then run the agent)."
    echo ""
    echo "3. Then run /tuicr again."
    exit 1
  fi

  # Check if tuicr is already running
  if check_tuicr_running "$target_dir"; then
    log_warn "tuicr is already running"
    log_info "Switch to its pane with Alt-arrow keys (default zellij binding)"
    exit 0
  fi

  # Launch tuicr in a split pane
  launch_tuicr_pane "$target_dir" "${TUICR_PASSTHROUGH_ARGS[@]+"${TUICR_PASSTHROUGH_ARGS[@]}"}"
}

main "$@"
