#!/usr/bin/env bash
set -e -u -o pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_tuicr-common.sh"

TUICR_PANE_DIRECTION="${TUICR_PANE_DIRECTION:-right}"
HERDR_BIN="${HERDR_BIN:-herdr}"
JQ_BIN="${JQ_BIN:-jq}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() {
  printf "%b[tuicr]%b %s\n" "$GREEN" "$NC" "$*"
}

log_warn() {
  printf "%b[tuicr]%b %s\n" "$YELLOW" "$NC" "$*"
}

log_error() {
  printf "%b[tuicr]%b %s\n" "$RED" "$NC" "$*" >&2
}

usage() {
  cat <<EOF
Usage: $(basename "$0") [directory] [-- tuicr-args...]

Launch tuicr in a Herdr split pane.

Arguments:
  directory    Repository directory to review (default: current directory)
  tuicr-args   Extra arguments passed through to tuicr (e.g. -w, -r <revset>)

Environment variables:
  TUICR_PANE_DIRECTION  Split direction: right or down (default: right)
  HERDR_BIN             Path to the Herdr executable (default: herdr)
  JQ_BIN                Path to the jq executable (default: jq)

Examples:
  $(basename "$0")
  $(basename "$0") ~/project
  $(basename "$0") . -- -w
  TUICR_PANE_DIRECTION=down $(basename "$0")
EOF
}

require_command() {
  local command_name="$1"
  local display_name="$2"

  if ! command -v "$command_name" &>/dev/null; then
    log_error "$display_name not found on PATH"
    return 1
  fi
}

new_pane_id=""

cleanup() {
  local status=$?

  if [[ -n "$new_pane_id" ]]; then
    "$HERDR_BIN" pane close "$new_pane_id" >/dev/null 2>&1 || true
  fi

  return "$status"
}

launch_tuicr_pane() {
  local target_dir="$1"
  shift
  local tuicr_args=("$@")

  case "$TUICR_PANE_DIRECTION" in
    right|down) ;;
    *)
      log_warn "Unknown TUICR_PANE_DIRECTION '$TUICR_PANE_DIRECTION'; using 'right'"
      TUICR_PANE_DIRECTION="right"
      ;;
  esac

  log_info "Launching tuicr in a Herdr pane split $TUICR_PANE_DIRECTION"
  log_info "Directory: $target_dir"

  local split_response
  split_response=$("$HERDR_BIN" pane split --current \
    --direction "$TUICR_PANE_DIRECTION" \
    --cwd "$target_dir" \
    --focus)

  new_pane_id=$(printf '%s\n' "$split_response" | \
    "$JQ_BIN" -er '.result.pane.pane_id')

  local tuicr_bin
  tuicr_bin=$(command -v tuicr)

  local quoted_tuicr
  printf -v quoted_tuicr '%q' "$tuicr_bin"
  quoted_tuicr="$quoted_tuicr$(tuicr_quote_args "${tuicr_args[@]+"${tuicr_args[@]}"}")"

  # Optional --stdout capture: skips tuicr's save & copy confirm dialog on
  # exit and writes the exported review straight to a file we print below.
  local output_file=""
  local use_stdout=false
  if tuicr_stdout_supported; then
    output_file=$(mktemp /tmp/tuicr-output.XXXXXX)
    quoted_tuicr="$quoted_tuicr --stdout > '$output_file'"
    use_stdout=true
    log_info "Using --stdout mode (output will be captured)"
  else
    log_warn "tuicr --stdout not supported, output will be copied to clipboard"
  fi

  local completion_suffix="_DONE_$$_${RANDOM}__"
  local completion_token="__TUICR${completion_suffix}"
  # `herdr pane run` injects this string into the pane's *interactive* shell,
  # which is not necessarily bash: `$?` and `VAR=value` are both parse errors
  # under fish. Wrap the payload in `bash -c` so the pane's shell only has to
  # forward one single-quoted argument -- single-quote semantics are identical
  # in bash, zsh and fish. bash (not sh) because $quoted_tuicr carries bash's
  # printf %q quoting, which can emit non-POSIX $'...' forms.
  # $completion_suffix needs no quoting: it is built from $$ and $RANDOM, so it
  # is digits and underscores by construction.
  local inner_script
  inner_script="$quoted_tuicr; printf \"\\n__TUICR%s:%s\\n\" $completion_suffix \$?"

  # $quoted_tuicr may itself contain a `'` (e.g. a -r revset with an
  # apostrophe, %q-escaped as \' for bash's own parsing). Embedding that raw
  # into the outer single-quoted argument below would close it early. Escape
  # every `'` as '\'' (close, escaped literal quote, reopen) so the pane's
  # shell reproduces $inner_script byte-for-byte as bash -c's one argument.
  local escaped_inner_script="${inner_script//\'/\'\\\'\'}"

  local pane_command
  pane_command="bash -c '$escaped_inner_script'"

  "$HERDR_BIN" pane run "$new_pane_id" "$pane_command" >/dev/null

  log_info "tuicr is running in pane $new_pane_id"
  log_info "Waiting for tuicr to exit..."

  local wait_response
  wait_response=$("$HERDR_BIN" pane wait-output "$new_pane_id" \
    --match "$completion_token" \
    --source recent-unwrapped)

  local matched_line
  matched_line=$(printf '%s\n' "$wait_response" | \
    "$JQ_BIN" -er '.result.matched_line')

  local tuicr_status="${matched_line##*:}"
  if [[ ! "$tuicr_status" =~ ^[0-9]+$ ]]; then
    log_error "Could not read tuicr exit status from Herdr output"
    return 1
  fi

  "$HERDR_BIN" pane close "$new_pane_id" >/dev/null
  new_pane_id=""

  if [[ "$tuicr_status" -eq 0 ]]; then
    log_info "tuicr finished"
  else
    log_error "tuicr exited with status $tuicr_status"
  fi

  tuicr_report_stdout_output "$use_stdout" "$output_file"

  return "$tuicr_status"
}

main() {
  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
  fi

  if [[ "${HERDR_ENV:-}" != "1" ]]; then
    log_error "Not running inside a Herdr-managed pane"
    echo "Run your coding agent inside Herdr, then invoke the tuicr skill again."
    exit 1
  fi

  require_command "$HERDR_BIN" "Herdr"
  require_command "$JQ_BIN" "jq"
  require_command "tuicr" "tuicr"

  tuicr_parse_args "$@"
  local target_dir="$TUICR_TARGET_DIR"
  if [[ ! -d "$target_dir" ]]; then
    log_error "Directory not found: $target_dir"
    exit 1
  fi
  target_dir=$(cd "$target_dir" && pwd)

  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM

  launch_tuicr_pane "$target_dir" "${TUICR_PASSTHROUGH_ARGS[@]+"${TUICR_PASSTHROUGH_ARGS[@]}"}"
}

main "$@"
