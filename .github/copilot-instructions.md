# tuicr — Copilot Instructions

tuicr is a Rust terminal UI application for reviewing diffs (AI-generated or otherwise) like a GitHub pull request, directly from the terminal.

## Build, test, lint

```bash
cargo build
cargo test
cargo fmt
cargo clippy -- -D warnings   # CI enforces -D warnings
```

Run a single test by name:
```bash
cargo test test_name_fragment   # matches any test whose name contains the fragment
```

## Architecture

### Central state: `App` (`src/app.rs`)

All UI state lives in `App`. It holds the VCS backend, the parsed diff files, the review session, scroll/cursor positions, and the current `InputMode`. Everything in the render and event loop reads from `App`.

### Input flow

```
crossterm Event → map_key_to_action(key, InputMode) → Action enum → handler in src/handler/
```

Each `InputMode` has its own handler (`handle_diff_action`, `handle_command_action`, etc.). Actions are defined in `src/input/keybindings.rs`.

### VCS abstraction (`src/vcs/`)

`VcsBackend` trait in `src/vcs/traits.rs` abstracts Git, Mercurial, and Jujutsu. `detect_vcs()` in `src/vcs/mod.rs` auto-detects (jj → git → hg). Git has two backends: libgit2 (default) and a CLI fallback used automatically for sparse checkouts, or forced via `backend = "cli"` in config.

`DiffFormat::Hg` and `DiffFormat::GitStyle` share the text parser in `src/vcs/diff_parser.rs`. Native Git uses libgit2 directly.

### Forge integration (`src/forge/`)

`ForgeBackend` trait in `src/forge/traits.rs`. Only GitHub is implemented (v1), via the `gh` CLI — not a REST/GraphQL SDK. All `gh` invocations go through `GhCommandRunner` in `src/forge/github/gh.rs`.

**Thread boundary rule**: network calls happen on a background thread; diff parsing and state mutations happen on the main thread. `SyntaxHighlighter` is not `Send`. Background threads return plain `Send`-safe data; `finish_*` functions on the main thread do the parsing.

### Persistence

Sessions are JSON files at `~/.local/share/tuicr/reviews/`. Loaded by `find_session_for_repo()` on startup. Config is at `~/.config/tuicr/config.toml` (XDG, or `%APPDATA%\tuicr\config.toml` on Windows). Unknown config keys produce startup warnings, not errors.

## Key conventions

### Error handling

`TuicrError` enum in `src/error.rs` using `thiserror`. All internal results use the `crate::error::Result<T>` type alias. Reserve `anyhow::Result` for `main()` and top-level CLI plumbing.

### Comment anchoring

A comment's line number lives in its `HashMap<u32, Vec<Comment>>` key — **not** in `Comment.line_context`. `Comment::new()` leaves `line_context: None`. Submit mapping takes an explicit `CommentAnchor` parameter; never infer file-level vs line-level from `line_context.is_none()`.

### Comment lifecycle

`CommentLifecycleState`: `LocalDraft` (editable) → `PushedDraft` or `Submitted` (locked). Locked comments cannot be edited or deleted locally. Old session JSON without this field rehydrates as `LocalDraft`.

### Session keys for PR reviews

`PrSessionKey { repository, number, head_sha }`. Same PR + same head SHA = same session (drafts reattach). New commit = new key = new session. Stale async results must be discarded by comparing this triple.

### `gh` API calls

Use `gh api --input -` (stdin) for payloads — never CLI args, which hit length limits for multi-comment payloads. On non-2xx, `gh api` writes the response body to stdout and a short status to stderr; the error formatter combines both.

### `gh pr diff`

Do **not** pass `--patch`. That flag produces per-commit mbox patches with duplicate file entries. Plain `gh pr diff` returns the cumulative diff.

### Updating docs after user-facing changes

| Document | Update when… |
|---|---|
| `README.md` | Keybindings, `:*` commands, CLI flags, features, installation |
| `src/ui/help_popup.rs` | Keybindings or commands (update `help_text` vec) |
| `AGENTS.md` | Module structure, key types, data flow, forge invariants/gotchas |

### Dogfooding

Run `cargo run` to review your own diff with tuicr before opening a PR. This is explicitly required by `CONTRIBUTING.md`.
