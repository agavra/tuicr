# tuicr — Copilot Instructions

tuicr is a Rust terminal UI application for reviewing diffs (AI-generated or otherwise) like a GitHub pull request, directly from the terminal.

The maintained agent guide is [`AGENTS.md`](../AGENTS.md) at the repository root: read it first. It covers the key invariants and gotchas, points to [`docs/INTERNALS.md`](../docs/INTERNALS.md) for subsystem notes, the testing rules (unit tests plus an e2e scenario for every user-visible change), and the table of docs to update after user-facing changes. Before touching `src/forge/`, PR mode, or PR session keys and submit, also read [`docs/FORGES.md`](../docs/FORGES.md).

## Build, test, lint

```bash
cargo build
cargo test
cargo fmt
cargo clippy -- -D warnings   # CI enforces -D warnings
```

End-to-end tests, Python checks and the dogfooding requirement are described in [`CONTRIBUTING.md`](../CONTRIBUTING.md) and [`e2e/README.md`](../e2e/README.md).
