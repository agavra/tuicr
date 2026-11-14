# e2e/lib/tmux.sh - the terminal driver: tuicr runs in a private tmux server, we send keys and read the screen.
# Every query that can fail (no such pane, capture error) fails the caller: a missing pane is never
# "text disappeared", "forbidden text absent" or "process exited".

_E2E_STARTED=()
_E2E_FORGE_LOGS=()

_tmux() { tmux -L "$E2E_SOCKET" -f /dev/null -u "$@"; }
_e2e_sess() { echo "e2e-$1"; }

# The tmux server exits when its last session ends, and a new-session sent to a server that is shutting down is lost
# (seen under load as "no pane" at the start of the next scenario). An idle anchor session keeps one server alive for the
# whole run, and remain-on-exit is set server-wide so even an instantly dying program leaves its pane (and exit status).
_e2e_anchor() {
  _tmux has-session -t e2e-anchor 2>/dev/null && return 0
  _tmux new-session -d -x "$E2E_COLS" -y "$E2E_ROWS" -s e2e-anchor -- sleep 86400 || return 1
  _tmux set-option -g remain-on-exit on
}

# e2e_start <name> <cmd...>   (cwd: $E2E_CWD, default <scratch>/work)
# Extra env for the child: E2E_ENV="K=V K2=V2"; E2E_PRE_PATH=<dir> goes first on PATH; E2E_KEEP=1 keeps the state dirs.
e2e_start() {
  local name=$1; shift
  local d; d=$(e2e_dir "$name")
  mkdir -p "$d"
  e2e_forge_unexpected_ok || E2E_FORGE_UNEXPECTED=1   # remember calls logged before the log is truncated
  [ -n "${E2E_KEEP:-}" ] || rm -rf "$d"/{home,config,data,state,tmp,work}
  mkdir -p "$d"/{home,config,data,state,tmp,work}  # always keeps $d/repo etc.
  : > "$d/forge.log"
  export E2E_FORGE_LOG="$d/forge.log"
  _E2E_FORGE_LOGS+=("$d/forge.log")
  local cwd=${E2E_CWD:-$d/work}
  _e2e_anchor || { echo "e2e: cannot start the tmux server on socket $E2E_SOCKET" >&2; return 1; }
  _tmux kill-session -t "$(_e2e_sess "$name")" 2>/dev/null || true
  # shellcheck disable=SC2086
  _tmux new-session -d -x "$E2E_COLS" -y "$E2E_ROWS" -s "$(_e2e_sess "$name")" -c "$cwd" -- \
    env -i HOME="$d/home" XDG_CONFIG_HOME="$d/config" XDG_DATA_HOME="$d/data" \
      XDG_STATE_HOME="$d/state" TMPDIR="$d/tmp" \
      PATH="${E2E_PRE_PATH:+$E2E_PRE_PATH:}$E2E_DIR/bin:$E2E_TOOL_PATH" \
      TERM=xterm-256color LANG="$E2E_LOCALE" LC_ALL="$E2E_LOCALE" \
      GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL="$d/home/.gitconfig" \
      E2E_FORGE_LOG="$d/forge.log" E2E_DATA="$E2E_DIR/data" \
      ${E2E_ENV:-} "$@" || return 1
  _E2E_STARTED+=("$name")
  _tmux has-session -t "$(_e2e_sess "$name")" 2>/dev/null || { echo "e2e: tmux session for '$name' vanished right after start" >&2; return 1; }
}

# e2e_start_keep: like e2e_start but keeps the scratch dirs (home/data/...) so a restart sees saved sessions.
e2e_start_keep() { E2E_KEEP=1 e2e_start "$@"; }

# e2e_keys <name> <tmux send-keys args...>
e2e_keys() { local n=$1; shift; _tmux send-keys -t "$(_e2e_sess "$n")" "$@"; }

# e2e_screen <name>: print the pane; non-zero when it cannot be captured (no such pane).
e2e_screen() { _tmux capture-pane -p -J -t "$(_e2e_sess "$1")" 2>/dev/null; }

# e2e_pane_exists <name> / e2e_alive <name> (exists and the process still runs)
e2e_pane_exists() { _tmux display-message -p -t "$(_e2e_sess "$1")" '#{pane_id}' >/dev/null 2>&1; }
e2e_alive() { [ "$(_tmux display-message -p -t "$(_e2e_sess "$1")" '#{pane_dead}' 2>/dev/null)" = 0 ]; }
# e2e_exit_status <name>: exit status of the finished process (empty while running / no pane).
e2e_exit_status() { _tmux display-message -p -t "$(_e2e_sess "$1")" '#{pane_dead_status}' 2>/dev/null; }

_e2e_no_pane() { echo "e2e: no pane for '$1' (tmux session gone or never started)" >&2; }

# e2e_wait_for <name> <regex> [timeout]  (ERE, whole screen). Polls every 100 ms.
e2e_wait_for() {
  local n=$1 re=$2 t=${3:-$E2E_TIMEOUT} i=0 scr
  while [ $i -lt $((t * 10)) ]; do
    scr=$(e2e_screen "$n") || { _e2e_no_pane "$n"; return 2; }
    grep -Eq -- "$re" <<<"$scr" && return 0
    sleep 0.1; i=$((i + 1))
  done
  echo "e2e: timed out after ${t}s waiting for /$re/ in '$n'. Screen:" >&2
  e2e_screen "$n" >&2
  return 1
}

# e2e_wait_gone <name> <regex> [timeout]: the pane exists and the text is absent.
e2e_wait_gone() {
  local n=$1 re=$2 t=${3:-$E2E_TIMEOUT} i=0 scr
  while [ $i -lt $((t * 10)) ]; do
    scr=$(e2e_screen "$n") || { _e2e_no_pane "$n"; return 2; }
    grep -Eq -- "$re" <<<"$scr" || return 0
    sleep 0.1; i=$((i + 1))
  done
  echo "e2e: /$re/ still on screen in '$n' after ${t}s" >&2; e2e_screen "$n" >&2; return 1
}

# e2e_wait_exit <name> [timeout] [expected-status|any]: wait for the process to end; its status must match (default 0).
e2e_wait_exit() {
  local n=$1 t=${2:-$E2E_TIMEOUT} want=${3:-0} i=0 dead st
  while [ $i -lt $((t * 10)) ]; do
    dead=$(_tmux display-message -p -t "$(_e2e_sess "$n")" '#{pane_dead}' 2>/dev/null) || { _e2e_no_pane "$n"; return 2; }
    if [ "$dead" = 1 ]; then
      st=$(e2e_exit_status "$n")
      # tmux 3.4 marks a pane dead (fd closed) a moment before it has collected the exit status: wait for it.
      for _ in $(seq 40); do [ -n "$st" ] && break; sleep 0.05; st=$(e2e_exit_status "$n"); done
      [ "$want" = any ] || [ "$st" = "$want" ] && return 0
      echo "e2e: '$n' exited with status ${st:-?}, expected $want. Screen:" >&2; e2e_screen "$n" >&2; return 1
    fi
    sleep 0.1; i=$((i + 1))
  done
  echo "e2e: '$n' did not exit within ${t}s" >&2; e2e_screen "$n" >&2; return 1
}

# e2e_settle <name> [quiet-seconds] [timeout]: wait until the screen has not changed for <quiet> (default 0.25 s).
# For "the UI has finished reacting to my keys" when nothing specific is to appear; prefer e2e_wait_for.
e2e_settle() {
  local n=$1 quiet=${2:-0.25} t=${3:-$E2E_TIMEOUT} prev cur stable=0 i=0 need
  need=$(awk -v q="$quiet" 'BEGIN { printf "%d", q * 20 + 0.5 }')   # polls every 50 ms
  prev=$(e2e_screen "$n") || { _e2e_no_pane "$n"; return 2; }
  while [ $i -lt $((t * 20)) ]; do
    sleep 0.05; i=$((i + 1))
    cur=$(e2e_screen "$n") || { _e2e_no_pane "$n"; return 2; }
    if [ "$cur" = "$prev" ]; then stable=$((stable + 1)); [ $stable -ge "$need" ] && return 0; else stable=0; prev=$cur; fi
  done
  echo "e2e: screen of '$n' never settled within ${t}s" >&2; return 1
}

# e2e_wait_screen_change <name> <previous-screen> [timeout-seconds, may be fractional]: the screen differs from a
# screen captured earlier (the barrier for "the app reacted", e.g. to an external file edit or a cursor key).
e2e_wait_screen_change() {
  local n=$1 before=$2 t=${3:-$E2E_TIMEOUT} i=0 cur max
  max=$(awk -v t="$t" 'BEGIN { printf "%d", t * 20 + 0.5 }')   # polls every 50 ms
  while [ $i -lt "$max" ]; do
    cur=$(e2e_screen "$n") || { _e2e_no_pane "$n"; return 2; }
    [ "$cur" != "$before" ] && return 0
    sleep 0.05; i=$((i + 1))
  done
  echo "e2e: screen of '$n' did not change within ${t}s" >&2; return 1
}

# e2e_colors_hash <name> [sed-range]: checksum of the screen WITH colours (to compare themes); range default = all rows.
e2e_colors_hash() { _tmux capture-pane -e -p -t "$(_e2e_sess "$1")" | sed -n "${2:-1,\$p}" | cksum; }

# e2e_resize <name> <cols> <rows>
e2e_resize() { _tmux resize-window -t "$(_e2e_sess "$1")" -x "$2" -y "$3"; }

# e2e_stop <name>: keep a screenshot + forge log in $E2E_OUT, kill the session.
e2e_stop() {
  local n=$1
  e2e_screen "$n" > "$E2E_OUT/$n.screen.txt" 2>/dev/null || true
  [ -s "$(e2e_dir "$n")/forge.log" ] && cp "$(e2e_dir "$n")/forge.log" "$E2E_OUT/$n.forge.log"
  _tmux kill-session -t "$(_e2e_sess "$n")" 2>/dev/null || true
}

e2e_stop_all() { local n; for n in "${_E2E_STARTED[@]:-}"; do [ -n "$n" ] && e2e_stop "$n"; done; return 0; }
