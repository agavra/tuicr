#!/usr/bin/env bash
set -e -u -o pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_tuicr-common.sh"

# Configuration - override via environment variables
TUICR_PANE_DIRECTION="${TUICR_PANE_DIRECTION:-right}"  # left, right, up or down
REX_BIN="${REX_BIN:-rex}"
JQ_BIN="${JQ_BIN:-jq}"

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

Launch tuicr in a Rex split block to review changes.

Arguments:
  directory    Git or jj repository directory to review (default: current directory)
  tuicr-args   Extra arguments passed through to tuicr (e.g. -w, -r <revset>)

Environment variables:
  TUICR_PANE_DIRECTION  Split direction: left, right, up or down (default: right)
  REX_BIN               Path to the rex executable (default: rex)
  JQ_BIN                Path to the jq executable (default: jq)

Examples:
  $(basename "$0")                       # Review changes in current directory
  $(basename "$0") ~/project             # Review changes in ~/project
  $(basename "$0") . -- -w               # Review uncommitted working-tree changes
  TUICR_PANE_DIRECTION=down $(basename "$0")
EOF
}

check_rex() {
  if ! command -v "$REX_BIN" &> /dev/null; then
    log_error "rex command not found on PATH"
    return 1
  fi
  if ! command -v "$JQ_BIN" &> /dev/null; then
    log_error "jq not found on PATH; the rex wrapper needs it to read rex's JSON output"
    return 1
  fi
  if [[ -z "${REX_SESSION:-}" || -z "${REX_BLOCK:-}" ]]; then
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

launch_tuicr_block() {
  local target_dir="$1"
  shift
  local tuicr_args=("$@")

  # rex names its directions right, left, below and above
  local rex_split
  case "$TUICR_PANE_DIRECTION" in
    right|left) rex_split="$TUICR_PANE_DIRECTION" ;;
    up) rex_split="above" ;;
    down) rex_split="below" ;;
    *)
      log_warn "Unknown TUICR_PANE_DIRECTION '$TUICR_PANE_DIRECTION', defaulting to 'right'"
      rex_split="right"
      ;;
  esac

  log_info "Launching tuicr in a $rex_split-split rex block"
  log_info "Directory: $target_dir"

  # Splitting the caller's own block (-b) keeps the review next to the agent
  # even when another block has focus. rex runs the command directly, with no
  # shell to quote for, and closes the block when tuicr exits. It returns at
  # once; --json because rex's plain output is not a stable format.
  local split_json
  split_json=$("$REX_BIN" split \
    --json \
    -s "$REX_SESSION" \
    -b "$REX_BLOCK" \
    -c "$target_dir" \
    --split "$rex_split" \
    --label tuicr \
    -- tuicr "${tuicr_args[@]+"${tuicr_args[@]}"}")

  # Take the one block ref in the response that is not the caller's own,
  # rather than depending on the name of the field that carries it.
  local block_ref
  block_ref=$("$JQ_BIN" -r --arg caller "$REX_BLOCK" \
    '[.. | strings | select(startswith("block:") and . != $caller)] | unique
     | if length == 1 then .[0] else empty end' \
    <<< "$split_json" 2>/dev/null || true)

  if [[ -z "$block_ref" ]]; then
    log_error "Could not read the new block ref from rex: $split_json"
    return 1
  fi

  log_info "tuicr is running in $block_ref"
  log_info "The block closes itself when tuicr exits; force it with: $REX_BIN block close $block_ref"
  echo ""
  echo "=== TUICR BLOCK ==="
  echo "$block_ref"
  echo "=== END TUICR BLOCK ==="
  echo ""
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
  tuicr_parse_args "$@"
  local target_dir="$TUICR_TARGET_DIR"
  target_dir=$(cd "$target_dir" && pwd)  # Get absolute path

  # Verify it's a git or jj repo
  if ! check_repo "$target_dir"; then
    exit 1
  fi

  # Check we're inside rex
  if ! check_rex; then
    log_error "Not running inside rex!"
    echo ""
    echo "To use tuicr with your coding agent, run that agent inside a rex block."
    echo "rex sets REX_SESSION and REX_BLOCK there."
    exit 1
  fi

  launch_tuicr_block "$target_dir" "${TUICR_PASSTHROUGH_ARGS[@]+"${TUICR_PASSTHROUGH_ARGS[@]}"}"
}

main "$@"
