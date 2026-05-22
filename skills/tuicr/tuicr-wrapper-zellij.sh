#!/usr/bin/env bash
set -e -u -o pipefail

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
Usage: $(basename "$0") [directory]

Launch tuicr in a zellij split pane to review git changes.

Arguments:
  directory    Git repository directory to review (default: current directory)

Environment variables:
  TUICR_PANE_DIRECTION  Split direction: down or right or stacked (default: stacked)
  ZELLIJ_BIN            Path to zellij executable

Examples:
  $(basename "$0")                            # Review changes in current directory
  $(basename "$0") ~/project                  # Review changes in ~/project
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

check_tuicr_stdout_support() {
  # Check if tuicr supports --stdout flag
  tuicr --help 2>&1 | grep -q -- '--stdout'
}

check_git_repo() {
  local dir="$1"
  if ! git -C "$dir" rev-parse --git-dir &> /dev/null; then
    log_error "Not a git repository: $dir"
    return 1
  fi
  return 0
}

check_tuicr_running() {
  # Zellij has no panes-list CLI like tmux. Fall back to a process check.
  if pgrep -x tuicr &> /dev/null; then
    return 0
  fi
  return 1
}

launch_tuicr_pane() {
  local target_dir="$1"

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
  local tuicr_cmd="$(command -v tuicr)"
  local use_stdout=false

  if check_tuicr_stdout_support; then
    output_file=$(mktemp /tmp/tuicr-output.XXXXXX)
    tuicr_cmd="$tuicr_cmd --stdout > '$output_file'"
    use_stdout=true
    log_info "Using --stdout mode (output will be captured)"
  else
    log_warn "tuicr --stdout not supported, output will be copied to clipboard"
  fi

  # Spawn tuicr in a new zellij pane. --close-on-exit cleans up the pane when
  # tuicr quits; the trailing FIFO write unblocks the wrapper.
  zellij_args=("--close-on-exit" "--name" "tuicr")

  if [[ "$TUICR_PANE_DIRECTION" == "stacked" ]]; then
    zellij_args+=("--stacked")
  else
    zellij_args+=("--direction" "$TUICR_PANE_DIRECTION")
  fi

  zellij_args+=(-- sh -c "$tuicr_cmd; echo done > '$fifo'")

  "$ZELLIJ_BIN" run\
    "${zellij_args[@]}"


  log_info "tuicr is running in a $TUICR_PANE_DIRECTION pane"
  log_info "Waiting for tuicr to exit..."

  # Block until the spawned command writes to the FIFO
  read -r _ < "$fifo"
  rm -f "$fifo"

  log_info "tuicr finished"

  # Output captured instructions if --stdout was used
  if [[ "$use_stdout" == true ]] && [[ -f "$output_file" ]]; then
    if [[ -s "$output_file" ]]; then
      echo ""
      echo "=== TUICR INSTRUCTIONS ==="
      cat "$output_file"
      echo "=== END TUICR INSTRUCTIONS ==="
    else
      log_info "No instructions exported from tuicr"
      log_info "If you exported to clipboard, paste the instructions here"
    fi
    rm -f "$output_file"
  else
    log_info "If you exported instructions, they are in your clipboard - paste them here"
  fi
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

  # Determine target directory
  local target_dir="${1:-.}"
  target_dir=$(cd "$target_dir" && pwd)  # Get absolute path

  # Verify it's a git repo
  if ! check_git_repo "$target_dir"; then
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
  if check_tuicr_running; then
    log_warn "tuicr is already running"
    log_info "Switch to its pane with Alt-arrow keys (default zellij binding)"
    exit 0
  fi

  # Launch tuicr in a split pane
  launch_tuicr_pane "$target_dir"
}

main "$@"
