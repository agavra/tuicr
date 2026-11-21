# AGENTS.md - tuicr

Two purposes: orient coding agents in the tuicr codebase, and explain how to read tuicr's review export (last section).

tuicr is a Rust TUI and library for code review; `src/lib.rs` is the public crate surface. Notes below are only what the code does not say. Read the matching doc before you touch:

- `src/forge/`, PR mode, PR sessions or submit, or code assigning `diff_files` in PR mode: **`docs/FORGES.md`** (contracts, async/session/submit rules, gotchas F*, G*).
- File tree filters, input modes and pickers, startup/themes/syntaxes/`tuicr update`, rendering and coloring, diff watch/reloads, session persistence, clipboard/export: **`docs/INTERNALS.md`**.
- e2e scenarios: **`e2e/README.md`** (API, reliability rules, fakes, debugging); areas in `e2e/COVERAGE.md`.

`skills/tuicr/` is the repository-managed agent skill (launches tuicr in a cmux, Rex, tmux, Zellij, or Herdr split pane).

## Testing rules

- Every user-visible bug fix or behaviour change adds or updates an e2e scenario `e2e/scenarios/<area>/<behaviour>.py`, named after the behaviour, with the PR/issue in its header comment. Unit tests are still required.
- **Red first.** Write the scenario before touching the code, run it on the unchanged parent and watch it go red, failing on the assertion that names the behaviour (not a typo, a missing fixture or a timeout in setup). Only then make the change and watch it go green. A scenario you never saw red proves nothing: it may be passing for the wrong reason. Put both results (the red failure line, then the green PASS) in the PR's test plan. For a new feature the red run is the scenario failing because the feature is absent.
- Run: `cargo build --release && TUICR_BIN=target/release/tuicr uv run --python 3.12 e2e/run.py [<area>/<behaviour>…]`. A new `e2e/scenarios/<area>/` needs a row in `e2e/COVERAGE.md`.
- Forges are always faked (`e2e/fakeforge/`); a scenario touches no real forge, network or credentials.
- Python changes pass the pinned ruff/ty checks in `CONTRIBUTING.md#python-checks` before commit.

## Key invariants and gotchas

Numbered once; forge/PR-mode gotchas (F*, G*) live in `docs/FORGES.md`.

1. **Patch display headers are never authoritative metadata.** Paths/statuses come from Git `--raw -z`, jj templates, Mercurial `status -0`, or forge JSON. `diff_parser` only consumes `@@` hunks, using their declared old/new counts as the grammar. Add no quoting or path-header heuristics.
2. **Anchor restore must scroll**: setting `diff_state.cursor_line` without `move_cursor_to_annotation` leaves the viewport at the top.
3. **`Comment`'s line anchor lives in the `HashMap` key** of `FileReview::line_comments`, not `Comment.line_context` (`Comment::new` leaves it unset). The submit mapper takes an explicit `CommentAnchor`; never infer "file-level" from `line_context.is_none()`.
4. **Comments are commit-scoped via `Comment::commit_id`.** With exactly one commit shown, `App::save_comment` stamps its SHA. `Some(sha)` comments are hidden when a different commit (or subset without `sha`) is selected; `None` (legacy, review-level, cumulative-diff) is always visible. `App::comment_visible(&Comment)` is the single predicate, applied in `rebuild_annotations`, both diff renderers, the navigator, and submit preflight. `AnnotatedLine::LineComment`/`FileComment` `comment_idx` is the **absolute** index into the stored value; `delete_comment_at_cursor` and `enter_edit_mode` look it up directly, never re-count by side.
5. **Register diff files before `r`, `R`, or a comment can land on them.** All three look the file up in `ReviewSession.files` by display path (toggles return silently; `add_comment_to_session` errors `session does not contain file`). Any code assigning `self.diff_files` must call `App::register_diff_files`.
6. **A filter is a view over `diff_files`, never a mutation**: `file_idx` stays absolute. `App::file_passes_filter()` is the single on-screen predicate, consulted by `build_visible_items`, `rebuild_annotations`, `file_render_height`/`effective_file_height` (0 lines when hidden), `hunk_positions`, and both diff renderers. Gate any new loop over `diff_files` with it.
7. **The `[TYPE]` prefix uses the config-resolved `label`**, not the `CommentType` id (`CommentType` has no label; `as_str()` is the uppercased id). `app::resolve_comment_type_label` is the single resolver (`resolve_comment_type_raw_label` gives the un-uppercased form for `{label}`); submitted prefixes render through `SubmitContext::type_tag` (`forge.comment_type_format`), which `describe_row` (`src/ui/submit_modals.rs`, the submit preview) reuses; `App::comment_type_label` and the markdown export delegate to it directly. Hence submit takes a `SubmitContext`, not a bare `&ForgeConfig`.
8. **PR mode parks the local backend.** PR mode swaps `vcs`/`vcs_info` for `PrNoopVcs` and a `forge:` root; the checkout's backend is parked in `App::parked_vcs` (or reopened with `detect_vcs` after `tuicr pr`). Anything that lists or loads a local target (selector, session resume) runs inside `with_local_vcs`, which swaps it in and then either restores the PR backend or, if a local review replaced the PR, drops the PR state. Scope local sessions by `local_repo_root`, never `vcs_info.root_path`.
9. **`VcsBackend::remote_url(name)`** (`src/vcs/traits.rs`) defaults to `UnsupportedOperation`; only Git overrides it. An adapter that adds it must also be wired into the Git-only discovery call (`--remote`) in `main.rs`; forge parsing stays in `forge::resolve_remote_repository`, never in the VCS adapter. `supports_sparse_checkout()` is the sparse capability hook.
10. **Global keys and redraw.** `map_global_key` (`src/input/keybindings.rs`) covers keys identical in every mode (`Ctrl-l` → `Redraw`); `main.rs` checks it via `handler::handle_global_key` before the pending-prefix, vim-editor and per-mode maps, so they never reach a text buffer. `Redraw` and `Event::Resize` call `App::request_full_redraw`; the main loop then calls `TerminalSession::repaint_from_scratch` inside the synchronized update. Ratatui's `Terminal::clear` stays out of that path: it queries the cursor position, which times out under `--stdout`.
11. **Gates run from the worktree**: `cd` into it before `cargo` (it resolves `Cargo.toml` from `pwd`; otherwise the wrong tree is exercised silently).

## Keeping docs updated

| Document | Update when adding/changing... |
| --- | --- |
| `README.md` | Features list, installation, essential keys, agent integration setup, forge table, critical config, docs index |
| `docs/CONFIG.md` | Any `config.toml` key: a row in Options and a line in Full example (only critical keys also go in `README.md`) |
| `docs/CLI.md` | Any clap flag, subcommand, env var, stderr marker, or exit code |
| `docs/KEYBINDINGS.md` | Any keybinding or `:` command: a row in the pane's key table and the `:` command table, prose for new behaviour |
| `src/ui/help_popup.rs` | Keybindings or commands (`help_text`) |
| `src/ui/status_bar.rs` | Keybindings worth advertising in the per-pane hint line |
| `docs/GITHUB.md` | GitHub PR behaviour: setup, targets, submit events, limitations |
| `docs/FORGES.md` | Forge backend contracts, detection, session/submit rules, F*/G* gotchas |
| `docs/INTERNALS.md` | Subsystem invariants and gotchas (non-forge) |
| `AGENTS.md` | Cross-cutting invariants and general gotchas, repo-managed agent integrations |
| `e2e/COVERAGE.md` | A new e2e scenario area, or closing a listed gap |
| `docs/LIBRARY.md` | Public library API (`ReviewStore`, `AddCommentRequest`); its Rust blocks compile as doctests |

---

## Review Format (for coding agents receiving tuicr output)

A user reviewing your changes with tuicr hands you a Markdown export like:

```markdown
I reviewed your code and have the following comments. Please address them.

Reviewing unstaged changes

Summary: {optional overall notes}

## Local tuicr Comments

1. `src/auth.rs:42` - Magic number should be a named constant
2. `src/api.rs` - Consider adding unit tests
```

Only the numbered comment list is guaranteed: the `[export]` section of `config.toml` can reword or drop the intro line, the `Reviewing …` line, the comment-type legend and either heading ([docs/CONFIG.md](docs/CONFIG.md)). PR mode adds `URL:` and `Head:` lines to the header, and unresolved upstream threads follow under their own heading.

Each item is `{n}. **[TYPE]** \`{location}\` - {content}`. The location is `{file}:{line}`, `{file}:{start}-{end}`, old-side (deleted) `{file}:~{line}` or `{file}:~{start}-~{end}`, or `{file}` for a file comment; a comment made against a single commit appends ` (commit {short_sha})`. Multi-line content and, with `[export] code_context`, a fenced `diff` block of the commented lines follow indented under the item.

`**[TYPE]** ` appears only when the user configured `comment_types` (it is the configured label, and a `Comment types: …` legend line precedes `Summary:`); untyped comments are read on their content. Common types: **[ISSUE]** blocks, fix it; **[SUGGESTION]** implement or say why not; **[NOTE]** answer or acknowledge; **[PRAISE]** needs no action.

Read the whole review, fix issues first, keep to the reviewer's intent, ask when a comment is unclear, and reply by comment number:

```markdown
## Changes Made

### Issues Addressed
- #1 - Extracted magic number to `TOKEN_EXPIRY_SECONDS` constant

### Suggestions Declined
- #2 - Kept current approach because [reason]

### Notes Addressed
- #3 - This approach was chosen for performance reasons
```
