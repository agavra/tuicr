# e2e/lib/flows.sh - UI flows shared by scenarios.
# sc_open <name> [tuicr args...]: fixture git repo, start tuicr with --no-update-check.
#   Default args: -r HEAD~2..HEAD (8 files, inline commit selector focused).
sc_open() {
  n=$1; shift; d=$(e2e_dir "$n"); mkdir -p "$d"; fx_git_repo "$d/repo"
  [ $# -gt 0 ] || set -- -r HEAD~2..HEAD
  E2E_CWD="$d/repo" e2e_start "$n" "$TUICR_BIN" --no-update-check "$@"
}
# sc_diff <name>: open the 2-commit range and move focus from the commit selector to the diff.
sc_diff() { sc_open "$1"; e2e_wait_for "$1" 'Files · 0/8'; e2e_keys "$1" Escape; }
# sc_quit <name>: force-quit and wait.
sc_quit() { e2e_keys "$1" Escape ":q!" Enter; e2e_wait_exit "$1"; }
# sc_cmd <name> <command>: run a ':' command.
sc_cmd() { e2e_keys "$1" Escape; e2e_keys "$1" ":$2" Enter; }
# sc_cursor_on <name> <regex>: j until the cursor row (▶) matches the regex (max 80 steps).
sc_cursor_on() {
  local n=$1 re=$2 i=0 scr rc cur
  scr=$(e2e_screen "$n") || { _e2e_no_pane "$n"; return 2; }
  while [ $i -lt 80 ]; do
    # (the cursor row is captured first: `grep ▶ | grep -q` fails under pipefail when -q exits early, i.e. exactly on a match)
    cur=$(grep -E '▶' <<<"$scr" || true); ! grep -Eq -- "$re" <<<"$cur" || return 0
    e2e_keys "$n" j
    # the cursor moving redraws the screen; at the end of the diff it cannot move, so stop waiting after 0.5 s
    rc=0; e2e_wait_screen_change "$n" "$scr" 0.5 2>/dev/null || rc=$?
    [ $rc -ne 2 ] || return 2
    e2e_settle "$n" 0.1 || return 2   # the first changed frame can be a partial redraw: judge the cursor row on a settled screen
    scr=$(e2e_screen "$n") || { _e2e_no_pane "$n"; return 2; }
    i=$((i+1))
  done
  echo "sc_cursor_on: cursor never reached /$re/" >&2; echo "$scr" >&2; return 1
}
# sc_session_json <name>: print the saved session json(s) of the scenario.
# (macOS stores them under $HOME/Library/Application Support, Linux under $XDG_DATA_HOME.)
sc_session_json() { find "$(e2e_dir "$1")/data" "$(e2e_dir "$1")/home" -path '*/sessions/*.json' -exec cat {} + 2>/dev/null; }
# sc_run <name> <args...>: run the tuicr binary non-interactively with the scenario's isolated env
# (E2E_RUN_PATH overrides the PATH; E2E_ENV adds variables).
sc_run() {
  local n=$1; shift; local d; d=$(e2e_dir "$n")
  mkdir -p "$d"/{home,config,data,state,tmp}
  # shellcheck disable=SC2086
  env -i HOME="$d/home" XDG_CONFIG_HOME="$d/config" XDG_DATA_HOME="$d/data" XDG_STATE_HOME="$d/state" TMPDIR="$d/tmp" \
    PATH="${E2E_RUN_PATH:-$E2E_DIR/bin:$E2E_TOOL_PATH}" E2E_DATA="$E2E_DIR/data" E2E_FORGE_LOG="$d/forge.log" \
    LANG="$E2E_LOCALE" LC_ALL="$E2E_LOCALE" ${E2E_ENV:-} "$TUICR_BIN" "$@"
}
# sc_capture <name> <args...>: run tuicr non-interactively (stdin closed); SC_OUT = stdout+stderr, SC_RC = exit status.
sc_capture() {
  local n=$1; shift
  SC_RC=0; SC_OUT=$(sc_run "$n" "$@" 2>&1 </dev/null) || SC_RC=$?
}
# e2e_minimal_path <dir> <tool...>: a PATH dir holding only the named tools (symlinks), for scenarios where the
# absence of a tool (az, gh, ...) must be deterministic rather than depend on the machine. Prints the dir.
e2e_minimal_path() {
  local dir=$1 t p; shift; mkdir -p "$dir"
  for t in "$@"; do p=$(command -v "$t") || { echo "e2e: $t not found" >&2; return 1; }; ln -sf "$p" "$dir/$t"; done
  echo "$dir"
}
# sc_save <name>: `:w` and wait for the confirmation ("Saved to <path>").
sc_save() { sc_cmd "$1" w; e2e_wait_for "$1" 'Saved to'; }
# sc_fake_clipboard <name>: put clipboard tools (pbcopy, xclip, wl-copy) that record stdin on the child's PATH, so a scenario
# neither touches the developer's clipboard nor depends on one existing. Prints nothing; read the copy with sc_clipboard.
# Call before e2e_start.
sc_fake_clipboard() {
  local n=$1 d b; d=$(e2e_dir "$n"); mkdir -p "$d/clipbin"; rm -f "$d/clipboard.txt"
  for b in pbcopy xclip wl-copy; do
    printf '#!/bin/sh\ncat > "%s"\n' "$d/clipboard.txt" > "$d/clipbin/$b"; chmod +x "$d/clipbin/$b"
  done
  E2E_PRE_PATH="$d/clipbin${E2E_PRE_PATH:+:$E2E_PRE_PATH}"; E2E_ENV="${E2E_ENV:-} XDG_SESSION_TYPE=x11"
}
# sc_clipboard <name>: wait for, then print, what the fake clipboard received.
sc_clipboard() { e2e_wait_file "$(e2e_dir "$1")/clipboard.txt" && cat "$(e2e_dir "$1")/clipboard.txt"; }
# sc_session_has <name> <regex>: a saved session mentions the pattern. The json is captured first: `sc_session_json | grep -q`
# under pipefail fails at random when grep exits before the writer finishes (SIGPIPE).
sc_session_has() { local j; j=$(sc_session_json "$1"); grep -Eq -- "$2" <<<"$j"; }
