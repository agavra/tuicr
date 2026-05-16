#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CAST_PATH="${ROOT_DIR}/public/tuicr-demo.cast"
GIF_PATH="${ROOT_DIR}/public/tuicr-demo.gif"

COLS="${DEMO_COLS:-144}"
ROWS="${DEMO_ROWS:-38}"
FONT_SIZE="${DEMO_FONT_SIZE:-12}"
LINE_HEIGHT="${DEMO_LINE_HEIGHT:-1.35}"
SPEED="${DEMO_SPEED:-1}"
PYTHON_BIN="${DEMO_PYTHON:-}"

usage() {
  cat <<'USAGE' >&2
Usage: scripts/demo/record-demo.sh

Records the README demo and overwrites:
  public/tuicr-demo.cast
  public/tuicr-demo.gif

Environment knobs:
  DEMO_COLS=144
  DEMO_ROWS=38
  DEMO_FONT_SIZE=12
  DEMO_LINE_HEIGHT=1.35
  DEMO_SPEED=1
  DEMO_PYTHON=python3
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -ne 0 ]]; then
  usage
  exit 2
fi

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: missing required command: $1" >&2
    exit 1
  fi
}

require asciinema
require agg
require cargo
require git
require pbcopy
require pbpaste

python_has_pexpect() {
  "$1" - <<'PY' >/dev/null 2>&1
try:
    import pexpect  # noqa: F401
except ImportError:
    raise SystemExit(1)
PY
}

if [[ -n "$PYTHON_BIN" ]]; then
  require "$PYTHON_BIN"
elif command -v python3 >/dev/null 2>&1 && python_has_pexpect python3; then
  PYTHON_BIN="python3"
elif command -v python3.11 >/dev/null 2>&1 && python_has_pexpect python3.11; then
  PYTHON_BIN="python3.11"
else
  echo "error: missing Python module: pexpect" >&2
  echo "install it with: python3 -m pip install pexpect" >&2
  echo "or set DEMO_PYTHON to a Python that already has pexpect" >&2
  exit 1
fi

if ! python_has_pexpect "$PYTHON_BIN"; then
  echo "error: $PYTHON_BIN cannot import pexpect" >&2
  echo "install it with: $PYTHON_BIN -m pip install pexpect" >&2
  exit 1
fi

run_dir="$(mktemp -d "${TMPDIR:-/tmp}/tuicr-demo-record.XXXXXX")"
cleanup() {
  rm -rf "$run_dir"
}
trap cleanup EXIT

fixture_dir="${run_dir}/fixture"
config_home="${run_dir}/config"
data_home="${run_dir}/data"

"${ROOT_DIR}/scripts/demo/setup-fixture.sh" "$fixture_dir" >/dev/null

mkdir -p "${config_home}/tuicr" "$data_home"
cat > "${config_home}/tuicr/config.toml" <<'EOF'
diff_view = "side-by-side"
show_file_list = true
wrap = false
mouse = false
leader = ";"
transparent_background = false
backend = "libgit2"
EOF

echo "Building tuicr..." >&2
cargo build --bin tuicr >&2

tuicr_bin="${ROOT_DIR}/target/debug/tuicr"
driver_cmd="$(printf '%q ' \
  "$PYTHON_BIN" \
  "${ROOT_DIR}/scripts/demo/drive_demo.py" \
  --tuicr "$tuicr_bin" \
  --fixture "$fixture_dir" \
  --cols "$COLS" \
  --rows "$ROWS")"

export XDG_CONFIG_HOME="$config_home"
export XDG_DATA_HOME="$data_home"
export TERM="xterm-256color"
export COLORTERM="truecolor"
unset NO_COLOR
export CLICOLOR_FORCE=1
export FORCE_COLOR=1

echo "Recording ${CAST_PATH}..." >&2
asciinema_help="$(asciinema rec --help 2>&1 || true)"
asciinema_args=(rec --overwrite --command "$driver_cmd")

if grep -q -- "--output-format" <<<"$asciinema_help"; then
  # agg 1.x expects asciicast v2.
  asciinema_args+=(--output-format asciicast-v2)
fi

if grep -q -- "--window-size" <<<"$asciinema_help"; then
  asciinema_args+=(--window-size "${COLS}x${ROWS}")
else
  asciinema_args+=(--cols "$COLS" --rows "$ROWS")
fi

if grep -q -- "--return" <<<"$asciinema_help"; then
  asciinema_args+=(--return)
fi

asciinema_args+=("$CAST_PATH")
asciinema "${asciinema_args[@]}"

if ! grep -Eq '38;(2|5)|48;(2|5)' "$CAST_PATH"; then
  echo "error: recording did not contain ANSI color escapes" >&2
  echo "hint: check that NO_COLOR is unset and the child terminal advertises truecolor" >&2
  exit 1
fi

echo "Rendering ${GIF_PATH}..." >&2
agg \
  --cols "$COLS" \
  --rows "$ROWS" \
  --font-size "$FONT_SIZE" \
  --line-height "$LINE_HEIGHT" \
  --speed "$SPEED" \
  --idle-time-limit 1.2 \
  --last-frame-duration 2 \
  --theme asciinema \
  "$CAST_PATH" \
  "$GIF_PATH"

echo "Wrote:" >&2
echo "  $CAST_PATH" >&2
echo "  $GIF_PATH" >&2
