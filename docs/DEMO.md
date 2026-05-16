# Recording the README Demo

The README gif is generated from an asciinema recording so it can be
re-recorded without manually stepping through tuicr.

## Prerequisites

On macOS:

```bash
brew install asciinema agg
python3 -m pip install pexpect
```

The recording also expects `cargo`, `git`, `pbcopy`, and `pbpaste`.

## Re-record

From the repository root:

```bash
scripts/demo/record-demo.sh
```

This overwrites:

- `public/tuicr-demo.cast`
- `public/tuicr-demo.gif`

The script builds `target/debug/tuicr`, generates a fresh temporary Rust
fixture repository, records tuicr with `--theme tokyo-night-storm`, and renders
the gif with `agg`. It clears `NO_COLOR` for the child recording so the cast
contains the same ANSI colors the app emits in a normal terminal.

## Demo Flow

The automated flow shows:

1. The startup commit selector.
2. Selecting two commits with `Space`, `Space`, `Enter`.
3. A side-by-side diff with the inline commit selector visible.
4. `/should_retry` search.
5. One suggestion line comment.
6. `:{N}` source-line jump.
7. One issue line comment.
8. `:clip`.
9. `pbpaste | sed -n '1,14p'` to prove the clipboard contains agent-ready
   markdown.

## Tuning

The wrapper defaults to roughly the README-friendly size we want:

```bash
DEMO_COLS=144
DEMO_ROWS=38
DEMO_FONT_SIZE=12
DEMO_LINE_HEIGHT=1.35
scripts/demo/record-demo.sh
```

`DEMO_SPEED` is also forwarded to `agg`. Set `DEMO_PYTHON` when `pexpect`
is installed for a non-default Python, for example:

```bash
DEMO_PYTHON=python3.11 scripts/demo/record-demo.sh
```

## Debugging

To inspect the fixture without recording:

```bash
fixture="$(scripts/demo/setup-fixture.sh)"
cd "$fixture"
git log --oneline
cargo test
```

To run just the driver after building tuicr:

```bash
cargo build --bin tuicr
python3 scripts/demo/drive_demo.py --tuicr target/debug/tuicr --fixture "$fixture"
```

The main recording path is macOS-oriented because the final proof step reads
from the real clipboard with `pbpaste`. In headless pseudo-terminals, tuicr may
use its OSC 52 fallback; the driver mirrors that payload into `pbcopy` before
printing the `pbpaste` preview. If the sandboxed recorder cannot access
`pbpaste`, it still previews the decoded OSC 52 content so the cast completes.
