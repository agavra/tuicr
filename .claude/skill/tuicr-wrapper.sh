#!/usr/bin/env bash
set -e -u -o pipefail

# Configuration - override via environment variables
TUICR_PANE_POSITION="${TUICR_PANE_POSITION:-top}"    # top or bottom
TUICR_PANE_SIZE="${TUICR_PANE_SIZE:-80}"              # percentage of screen

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

Launch tuicr in a tmux split pane to review git changes.

Arguments:
  directory    Git repository directory to review (default: current directory)

Environment variables:
  TUICR_PANE_POSITION   Position of tuicr pane: top or bottom (default: top)
  TUICR_PANE_SIZE       Size of pane as percentage (default: 80)

Examples:
  $(basename "$0")                    # Review changes in current directory
  $(basename "$0") ~/project          # Review changes in ~/project
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
  # Check if tuicr is already running in any tmux pane
  if tmux list-panes -a -F '#{pane_current_command}' 2>/dev/null | grep -q '^tuicr$'; then
    return 0  # tuicr is running
  fi
  return 1
}

launch_tuicr_pane() {
  local target_dir="$1"

  # Get window height and calculate lines (using -l instead of -p to avoid "size missing" error)
  local window_height
  window_height=$(tmux display-message -p '#{window_height}')
  local pane_lines=$(( window_height * TUICR_PANE_SIZE / 100 ))

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

  log_info "Launching tuicr in $TUICR_PANE_POSITION pane (${pane_lines} lines, ${TUICR_PANE_SIZE}%)"
  log_info "Directory: $target_dir"

  # Create unique channel for wait-for
  local wait_channel="tuicr-$$"

  # Check if --stdout is supported and set up output capture
  local output_file=""
  local tuicr_cmd="tuicr"
  local use_stdout=false

  if check_tuicr_stdout_support; then
    output_file=$(mktemp /tmp/tuicr-output.XXXXXX)
    tuicr_cmd="tuicr --stdout > '$output_file'"
    use_stdout=true
    log_info "Using --stdout mode (output will be captured)"
  else
    log_warn "tuicr --stdout not supported, output will be copied to clipboard"
  fi

  # Create the split pane with tuicr, signal when done
  # Use -d to not switch, -P to print pane info so we can capture the ID
  local new_pane_id
  new_pane_id=$(tmux split-window -d -P -F '#{pane_id}' "${split_args[@]}" \
    "cd '$target_dir' && $tuicr_cmd; tmux wait-for -S '$wait_channel'")

  # Switch focus to the new tuicr pane
  tmux select-pane -t "$new_pane_id"

  log_info "tuicr is running in pane $new_pane_id"
  log_info "Waiting for tuicr to exit..."

  # Block until tuicr exits
  tmux wait-for "$wait_channel"

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

  # Check if we're in tmux
  if ! check_tmux; then
    log_error "Not running inside tmux!"
    echo ""
    echo "To use tuicr with Claude Code, you need to run Claude inside tmux."
    echo ""
    echo "1. Exit Claude now:"
    echo "   Type /exit or press Ctrl+C twice"
    echo ""
    echo "2. Restart Claude inside tmux (will resume this session):"
    echo "   tmux new-session 'claude -c'"
    echo ""
    echo "3. Then run /tuicr again"
    exit 1
  fi

  # Check if tuicr is already running
  if check_tuicr_running; then
    log_warn "tuicr is already running in another pane"
    log_info "Switch to it with Ctrl-b + arrow keys"
    exit 0
  fi

  # Launch tuicr in a split pane
  launch_tuicr_pane "$target_dir"
}

main "$@"
