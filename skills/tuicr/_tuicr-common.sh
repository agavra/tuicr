#!/usr/bin/env bash
# Sourced by the tuicr-wrapper-*.sh scripts. Not an entry point on its own.

# Splits "$@" on the `[directory] [-- tuicr-args...]` convention every
# wrapper shares. Sets TUICR_TARGET_DIR and the TUICR_PASSTHROUGH_ARGS array
# instead of returning them, since a sourced function's positional
# parameters are its own and can't shift the caller's.
tuicr_parse_args() {
  TUICR_TARGET_DIR="."
  if [[ "${1:-}" != "--" && -n "${1:-}" ]]; then
    TUICR_TARGET_DIR="$1"
    shift
  fi
  if [[ "${1:-}" == "--" ]]; then
    shift
  fi
  TUICR_PASSTHROUGH_ARGS=("$@")
}

# Prints each argument bash-%q-quoted with a leading space, e.g.
# `tuicr_quote_args -w` prints " -w", so the caller can append the result
# straight onto a command string: cmd="tuicr$(tuicr_quote_args "${args[@]}")"
tuicr_quote_args() {
  local arg quoted
  for arg in "$@"; do
    printf -v quoted ' %q' "$arg"
    printf '%s' "$quoted"
  done
}

# True if the installed tuicr accepts --stdout, which exports the review
# straight to a file instead of prompting the human to save & copy on exit.
tuicr_stdout_supported() {
  tuicr --help 2>&1 | grep -q -- '--stdout'
}

# Prints the review exported via --stdout, or points the caller at the
# clipboard when --stdout capture wasn't used. Cleans up output_file either
# way. Callers must define log_info before invoking this.
tuicr_report_stdout_output() {
  local use_stdout="$1"
  local output_file="$2"

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
