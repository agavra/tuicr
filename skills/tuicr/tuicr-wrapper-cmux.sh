#!/usr/bin/env bash
set -e -u -o pipefail

# Configuration - override via environment variables
TUICR_PANE_DIRECTION="${TUICR_PANE_DIRECTION:-right}"  # left, right, up or down
CMUX_BIN="${CMUX_BIN:-cmux}"

# Backward-compat: map old tmux-style position to a cmux direction
case "${TUICR_PANE_POSITION:-}" in
  top) TUICR_PANE_DIRECTION="up" ;;
  bottom) TUICR_PANE_DIRECTION="down" ;;
  left|right) TUICR_PANE_DIRECTION="$TUICR_PANE_POSITION" ;;
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

Launch tuicr in a cmux split pane to review changes.

Arguments:
  directory    Git or jj repository directory to review (default: current directory)
  tuicr-args   Extra arguments passed through to tuicr (e.g. -w, -r <revset>)

Environment variables:
  TUICR_PANE_DIRECTION  Split direction: left, right, up or down (default: right)
  CMUX_BIN              Path to the cmux executable (default: cmux)

Examples:
  $(basename "$0")                       # Review changes in current directory
  $(basename "$0") ~/project             # Review changes in ~/project
  $(basename "$0") . -- -w               # Review uncommitted working-tree changes
  TUICR_PANE_DIRECTION=down $(basename "$0")
EOF
}

check_cmux() {
  if ! command -v "$CMUX_BIN" &> /dev/null; then
    log_error "cmux command not found on PATH"
    return 1
  fi
  if [[ -z "${CMUX_WORKSPACE_ID:-}" ]]; then
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

# ponytail: no already-running guard. cmux workspaces are independent contexts and
# a global `pgrep -x tuicr` blocks a legitimate review whenever another workspace
# has one open. A stray pane costs nothing; `cmux list-panes` finds it.

launch_tuicr_pane() {
  local target_dir="$1"
  shift
  local tuicr_args=("$@")

  case "$TUICR_PANE_DIRECTION" in
    left|right|up|down) ;;
    *)
      log_warn "Unknown TUICR_PANE_DIRECTION '$TUICR_PANE_DIRECTION', defaulting to 'right'"
      TUICR_PANE_DIRECTION="right"
      ;;
  esac

  log_info "Launching tuicr in a $TUICR_PANE_DIRECTION-split cmux pane"
  log_info "Directory: $target_dir"

  # `cmux new-pane` prints: OK surface:<n> pane:<n> workspace:<n>
  local create_output
  create_output=$("$CMUX_BIN" new-pane \
    --type terminal \
    --direction "$TUICR_PANE_DIRECTION" \
    --focus true)

  local surface_ref
  surface_ref=$(printf '%s\n' "$create_output" | grep -o 'surface:[0-9]*' | head -1)

  if [[ -z "$surface_ref" ]]; then
    log_error "Could not read the new surface id from cmux: $create_output"
    return 1
  fi

  # A new pane starts a login shell in the workspace cwd; cd explicitly so the
  # wrapper works regardless of which directory that turns out to be.
  local command_line
  # exec: no shell survives tuicr, so cmux closes the pane when the TUI exits
  printf -v command_line 'cd %q && exec tuicr' "$target_dir"
  local arg
  for arg in ${tuicr_args[@]+"${tuicr_args[@]}"}; do
    printf -v command_line '%s %q' "$command_line" "$arg"
  done

  "$CMUX_BIN" send --surface "$surface_ref" "$command_line"$'\n' > /dev/null

  log_info "tuicr is running in $surface_ref"
  log_info "The pane closes itself when tuicr exits; force it with: $CMUX_BIN close-surface --surface $surface_ref"
  echo ""
  echo "=== TUICR SURFACE ==="
  echo "$surface_ref"
  echo "=== END TUICR SURFACE ==="
  echo ""
  # ponytail: returns immediately — cmux has no tmux-style `wait-for`, and the
  # documented agent loop is polling `tuicr review comments` anyway. Add a FIFO
  # handshake (as the zellij wrapper does) only if a blocking launch is needed.
  log_info "Not waiting for exit. Read feedback with: tuicr review comments --repo $target_dir --session <slug>"
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
  local target_dir="."
  if [[ "${1:-}" != "--" && -n "${1:-}" ]]; then
    target_dir="$1"
    shift
  fi
  if [[ "${1:-}" == "--" ]]; then
    shift
  fi
  target_dir=$(cd "$target_dir" && pwd)  # Get absolute path

  # Verify it's a git or jj repo
  if ! check_repo "$target_dir"; then
    exit 1
  fi

  # Check we're inside cmux
  if ! check_cmux; then
    log_error "Not running inside cmux!"
    echo ""
    echo "To use tuicr with your coding agent, run that agent inside cmux."
    echo ""
    echo "1. Open the repository in cmux."
    echo ""
    echo "2. Start the agent from a cmux terminal."
    echo ""
    echo "3. Then run /tuicr again."
    exit 1
  fi

  launch_tuicr_pane "$target_dir" "$@"
}

main "$@"
