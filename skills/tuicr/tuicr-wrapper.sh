#!/usr/bin/env bash
set -e -u -o pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_tuicr-common.sh"

# Configuration - override via environment variables
TUICR_PANE_POSITION="${TUICR_PANE_POSITION:-top}"    # top or bottom
TUICR_PANE_SIZE="${TUICR_PANE_SIZE:-80}"              # percentage of the calling pane

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

Launch tuicr in a tmux split pane to review changes.

Arguments:
  directory    Git or jj repository directory to review (default: current directory)
  tuicr-args   Extra arguments passed through to tuicr (e.g. -w, -r <revset>)

Environment variables:
  TUICR_PANE_POSITION   Position of tuicr pane: top or bottom (default: top)
  TUICR_PANE_SIZE       Size of pane as percentage of the calling pane (default: 80)

Examples:
  $(basename "$0")                    # Review changes in current directory
  $(basename "$0") ~/project          # Review changes in ~/project
  $(basename "$0") . -- -w            # Review uncommitted working-tree changes
  TUICR_PANE_SIZE=70 $(basename "$0") # Use 70% of screen
EOF
}

check_tmux() {
  if [[ -z "${TMUX:-}" ]]; then
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

check_tuicr_running() {
  # Check if tuicr is already running in any tmux pane
  if tmux list-panes -a -F '#{pane_current_command}' 2>/dev/null | grep -q '^tuicr$'; then
    return 0  # tuicr is running
  fi
  return 1
}

launch_tuicr_pane() {
  local target_dir="$1"
  shift
  local tuicr_args=("$@")

  # Split the pane this wrapper runs in, not whichever pane is focused. Without
  # an explicit target tmux uses the client's active pane — the one the human
  # is looking at — so an agent's review lands in someone else's pane and
  # blocks it. tmux sets $TMUX_PANE for every pane it spawns.
  local caller_pane="${TMUX_PANE:-}"
  if [[ -z "$caller_pane" ]]; then
    caller_pane=$(tmux display-message -p '#{pane_id}')
  fi

  # Get the caller's height and calculate lines (using -l instead of -p to avoid "size missing" error)
  local pane_height
  pane_height=$(tmux display-message -p -t "$caller_pane" '#{pane_height}')
  local pane_lines=$(( pane_height * TUICR_PANE_SIZE / 100 ))

  # Build the split-window command
  local split_args=()

  # Determine split direction based on position
  if [[ "$TUICR_PANE_POSITION" == "top" ]]; then
    split_args+=(-b)  # Create pane above
  fi
  # For bottom, no -b flag needed (default)

  # Set pane size in lines (not percentage, to work without TTY)
  split_args+=(-l "$pane_lines")

  # Change to target directory
  split_args+=(-c "$target_dir")

  log_info "Launching tuicr in $TUICR_PANE_POSITION pane of $caller_pane (${pane_lines} lines, ${TUICR_PANE_SIZE}%)"
  log_info "Directory: $target_dir"

  # Create unique channel for wait-for
  local wait_channel="tuicr-$$"

  # Check if --stdout is supported and set up output capture
  local output_file=""
  local tuicr_cmd="tuicr$(tuicr_quote_args "${tuicr_args[@]+"${tuicr_args[@]}"}")"
  local use_stdout=false

  if tuicr_stdout_supported; then
    output_file=$(mktemp /tmp/tuicr-output.XXXXXX)
    tuicr_cmd="$tuicr_cmd --stdout > '$output_file'"
    use_stdout=true
    log_info "Using --stdout mode (output will be captured)"
  else
    log_warn "tuicr --stdout not supported, output will be copied to clipboard"
  fi

  # Create the split pane with tuicr, signal when done
  # Use -d to not switch, -P to print pane info so we can capture the ID
  local new_pane_id
  new_pane_id=$(tmux split-window -d -P -F '#{pane_id}' -t "$caller_pane" "${split_args[@]}" \
    "cd '$target_dir' && $tuicr_cmd; tmux wait-for -S '$wait_channel'")

  # Focus the new pane only when the caller already has focus in its window.
  # If the human is working in another pane, leave them alone — they will
  # come back to the agent's pane (e.g. via its notification) and find the
  # review waiting next to it.
  if [[ "$(tmux display-message -p -t "$caller_pane" '#{pane_active}')" == "1" ]]; then
    tmux select-pane -t "$new_pane_id"
  else
    log_info "Caller pane is not focused; leaving focus where it is"
  fi

  log_info "tuicr is running in pane $new_pane_id"
  log_info "Waiting for tuicr to exit..."

  # Block until tuicr exits
  tmux wait-for "$wait_channel"

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

  # Check if we're in tmux
  if ! check_tmux; then
    log_error "Not running inside tmux!"
    echo ""
    echo "To use tuicr with your coding agent, run that agent inside tmux."
    echo ""
    echo "1. Exit the current agent session."
    echo ""
    echo "2. Restart the agent inside tmux."
    echo ""
    echo "3. Then run /tuicr again."
    exit 1
  fi

  # Check if tuicr is already running
  if check_tuicr_running; then
    log_warn "tuicr is already running in another pane"
    log_info "Switch to it with Ctrl-b + arrow keys"
    exit 0
  fi

  # Launch tuicr in a split pane
  launch_tuicr_pane "$target_dir" "${TUICR_PASSTHROUGH_ARGS[@]+"${TUICR_PASSTHROUGH_ARGS[@]}"}"
}

main "$@"
