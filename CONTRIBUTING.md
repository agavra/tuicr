# Contributing

## The workflow

1. Use Codex, Claude, or your favorite AI coding agent to make changes
2. Run `cargo run` to review the diff with `tuicr` itself
3. Export your comments and feed them back to the agent
4. Repeat until you're happy
5. Open a PR

Dogfooding is mandatory. If `tuicr` is annoying to use for your own changes, fix that first.

## Building

```bash
cargo build
cargo test
cargo fmt
```

## Testing

`cargo test` runs the unit and integration tests. User-visible bug fixes and behaviour changes also need an
end-to-end scenario under `e2e/scenarios/<area>/<behaviour>.sh` that fails without the change:

```bash
cargo build --release
TUICR_BIN=target/release/tuicr e2e/run.sh            # or filter: e2e/run.sh pr/submit
```

Prerequisites: the Rust toolchain, python3 >= 3.9, tmux, [uv](https://docs.astral.sh/uv/) (provides `uvx` for the
Python checks below), plus `hg` and `jj` for their scenarios (they skip without them). Forges are
faked; no credentials or network needed. CI runs the suite on Linux and macOS. See [e2e/README.md](e2e/README.md)
and [e2e/COVERAGE.md](e2e/COVERAGE.md).

## Python checks

All Python in the repo (the e2e suite, its fake forges, `scripts/`) is linted, formatted and type-checked; config is
`e2e/pyproject.toml` (Python 3.11 target). Run these from the repo root before committing Python changes. The
versions are pinned to match CI (`RUFF_VERSION` / `TY_VERSION` in `.github/workflows/ci.yml`; bump both together):

```bash
uvx ruff@0.15.16 format --check --config e2e/pyproject.toml e2e scripts/demo   # drop --check to rewrite
uvx ruff@0.15.16 check --config e2e/pyproject.toml e2e scripts/demo           # add --fix for safe autofixes
(cd e2e && uvx ty@0.0.44 check . ../scripts/demo)
```

CI runs `format --check` in the Format job and `check` plus `ty check` in the Clippy job.

## Guidelines

- Keep it simple—this is a focused tool, not a platform
- If you add a feature, use it yourself first
- Bug reports and feature requests welcome via issues

## Committers

The following individuals have write access to `tuicr`:

- [@agavra](https://github.com/agavra) (chair)
- [@YPares](https://github.com/YPares)
- [@martintrojer](https://github.com/martintrojer)

The chair has final decision-making authority on project direction. Consistent,
quality contributions (issues, pull request reviews and code submissions) are
the path to committership, keep contributing and you may receive an invite!
