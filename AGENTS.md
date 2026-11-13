# AGENTS.md - tuicr

Two purposes: orient coding agents in the tuicr codebase, and explain how to read tuicr's review export (last section).

tuicr is a Rust TUI and library for code review. `src/` layout is self-describing (`ls src/`); `src/lib.rs` is the public crate surface (`ReviewStore`, session/comment types). Notes below are only what the code does not say.

Repository-managed agent integration: `skills/tuicr/` (shared skill bundle that launches tuicr in a cmux, Rex, tmux, Zellij, or Herdr split pane).

## Key types and invariants

**App** (`src/app/mod.rs`, `src/app/`): central state (`vcs`, `vcs_info`, `session`, `diff_files`, `input_mode`, scroll/cursor). PR mode adds `pr_info`. Comment navigation uses `CommentNavigatorState`/`CommentNavigatorItem` derived from `line_annotations`, so local comments and visible remote threads jump to the annotations the diff renders (`move_cursor_to_annotation`).

**VcsBackend** (`src/vcs/traits.rs`): `GitBackend` (libgit2 by default; Git CLI for sparse checkouts, which also emits a startup warning, or `backend = "cli"`), `HgBackend`, `JjBackend`, all always compiled. `detect_vcs()` tries jj, then git, then hg. `supports_sparse_checkout()` is a capability hook. `remote_url(name)` must be implemented by every adapter and wired into the Git-only discovery call in `main.rs`; forge parsing stays in `forge::resolve_remote_repository`, never in the VCS adapter.

**FileTreeFilter** (`src/app/mod.rs`, `src/app/file_filter.rs`): `i`/`e` regex include/exclude, `/` path search, and `show_reviewed` (`:set reviewed!`, config `show_reviewed`).
- `show_reviewed` is **command-only**: the project reserves single-stroke keys for vim motions, so never add a key for it.
- A filter is a *view* over `diff_files`, never a mutation: `file_idx` stays absolute. `App::file_passes_filter()` is the single on-screen predicate, consulted by `build_visible_items`, `rebuild_annotations`, `file_render_height`/`effective_file_height` (0 lines when hidden), `hunk_positions`, and both diff renderers. Gate any new loop over `diff_files`.
- Two predicates on purpose: `file_matches_patterns()` (`i`/`e` only: the review *population*, used by `file_count()`, `reviewed_count()`, `/` search) vs `file_passes_filter()` (that plus `show_reviewed`). Counting visible rows would render the progress fraction as `0/n` when reviewed files are hidden. Only the file-level `reviewed` flag hides; a file with individually `R`-marked hunks stays visible (and `is_hunk_reviewed` hashes every hunk, so do not call it speculatively).
- `FileTreeFilter` has a hand-written `Default` because `show_reviewed` defaults to *true*.
- Marking a file reviewed while hiding deletes the cursor's row, so `toggle_reviewed_for_file_idx` calls `advance_past_hidden_file()` (next visible file, wrapping, parking at the overview when empty) regardless of `adjust_cursor`.
- `file_filter.draft` makes the tree a text-input sub-state of `InputMode::Normal` (same shape as `pr_filter_draft`); `main.rs` routes to `map_file_tree_prompt_mode` while it is `Some`. An open prompt owns input whatever panel has focus (a click can move focus to the diff): `handler::handle_normal_action` checks it before `focused_panel`, and `handler::handle_paste` is the one paste table (prompts and picker filters accept paste; other Normal-mode paste is dropped on purpose). A diff drag is ignored while it is open (no `VisualSelect`). `main.rs` routes keys through `handler::map_normal_path_key` and `handler::dispatch_action`, so tests drive the real path. Keys are focus-scoped via `map_file_tree_mode`: the tree claims `i`/`e`/`I`/`E`/`/`; the diff keeps `i` = edit comment, `/` = search diff.
- Filters are session-local, never persisted, and apply at render/measure time, unlike `.tuicrignore`, which drops files at load time.

**InputMode** (`src/app/mod.rs`): `Normal`, `Command`, `Comment` (Ctrl-S saves, Ctrl-C cancels), `Search`, `Help`, `MessageDetails`, `Summary`, `Confirm`, `CommitSelect`, `VisualSelect`, `ThemePicker`, `FilePicker`.
- `Summary` (`:summary`) replaces the diff, keeps the file sidebar, lists every `local_draft` comment, selects the first on open. `Enter` jumps to the comment in the continuous diff (leaving single-file view; a reviewed file/hunk is revealed without clearing its persisted state); `Esc` returns without jumping.
- `ThemePicker` (`src/app/theme_picker.rs`): `j`/`k` live-previews by reassigning `App::theme` (renderers read `&app.theme` per frame, so no invalidation). `/` opens a filter draft committed on `Enter`, discarded on `Esc`. `Enter` keeps the preview for the session and never writes `config.toml`; `Esc` reverts to a `Theme` snapshot taken on entry (`Theme` has a manual `Clone` because its `OnceLock` highlighter cache is not `Clone`). `:theme <name>` applies directly.
- `FilePicker` (`src/app/file_picker.rs`, `<leader>p`): built like `ThemePicker`, but the list narrows while the filter draft is typed and `Enter` jumps straight to the selection (`jump_to_file`). Candidates are captured on open from files passing `file_passes_filter`; a background reload can replace `diff_files` meanwhile, so `Enter` re-resolves by display path and re-checks the filter.

**ReviewSession** (`src/model/review.rs`): persisted state, `files: HashMap<PathBuf, FileReview>` (`reviewed`, `reviewed_hunks`, `file_comments`, `line_comments: HashMap<u32, Vec<Comment>>`). PR sessions add `pr_description_reviewed` (`r` on the PR description): when set, `ui::pr_info_panel::pr_info_display_lines` yields only the title row, and every PR-info row counter (annotations, heights, search) must go through it to match the renderer. It returns rows cached on `App::pr_info_rows_cache` (keyed on width and PR metadata); swap themes through `App::set_theme` to drop the cache, and both diff renderers sync the inner width before building these rows. It merges three-way like a file's `reviewed` and carries over unconditionally to a new PR head.

**ReviewStore** (`src/review_store.rs`): library facade (`list_sessions_for_repo`, `get_review`, `add_comment`, `save_review`, `delete_review`). `add_comment_to_session()` is shared with `App::save_comment()`.

**PR mode and the local backend**: PR mode swaps `vcs`/`vcs_info` for `PrNoopVcs` and a `forge:` root; the checkout's backend is parked in `App::parked_vcs` (or reopened with `detect_vcs` after `tuicr pr`). Anything that lists or loads a local target (selector, session resume) must run inside `with_local_vcs`, which swaps it in and then either restores the PR backend or, if a local review replaced the PR, drops the PR state. Scope local sessions by `local_repo_root`, never `vcs_info.root_path`.

**Action** (`src/input/keybindings.rs`): `map_key_to_action(key, mode)`. `map_global_key` covers keys identical in every mode (`Ctrl-l` → `Redraw`); `main.rs` checks it via `handler::handle_global_key` before the pending-prefix, vim-editor and per-mode maps, so they never reach a text buffer. `Redraw` and `Event::Resize` call `App::request_full_redraw`; the main loop then calls `TerminalSession::repaint_from_scratch` inside the synchronized update. Never use ratatui's `Terminal::clear` there: it queries the cursor position, which times out under `--stdout`.

## Behavior notes (not visible from the code layout)

- **Startup**: invalid `--theme` exits non-zero; precedence `--theme` > config > dark. Theme lookup: bundled, then `$XDG_CONFIG_HOME/tuicr/themes/` (`%APPDATA%` on Windows); local themes may reference a `.tmTheme`, and `src/syntax/mod.rs` translates the ANSI-slot placeholders some bat-compatible Base16 files use. `syntax::load_user_syntaxes` adds sibling `syntaxes/*.sublime-syntax` once, before the first `SyntaxHighlighter` exists (absent/empty dir skips the ~200ms relink; a bad grammar is a startup warning). Unknown config keys warn and are ignored. `tuicr update` exits before TUI setup (flow in `src/update/`).
- **`--line N`** (needs `--file`/`--path`, rejected with `pr`): `App::pending_start_line` is applied by `apply_pending_start_line` after the first `terminal.draw`, because centering needs the viewport height render measures; it waits while a modal/selector is open or no rows exist. `main.rs` repaints in the same synchronized update so the unpositioned frame never shows.
- **Sidebar position**: with `file_list_position = "right"`, `App::focus_pane_left`/`focus_pane_right` mirror the `<leader>h`/`<leader>l` walk; mouse hit-testing reads the rendered `file_list_area`/`diff_area` rects, so it needs no change.
- **Selection colors**: local-theme `selection_marker` drives cursor indicators and commit-range bars independently of `border_focused`; missing values keep each indicator's legacy foreground; row backgrounds still use `bg_highlight`/`cursor_line_bg`, PR numbers/hashes `cursor_color`.
- **Diff watch**: `diff_watch_interval_ms` reload also refreshes the inline commit pane and the staged/unstaged rows; a `DiffSource::CommitRange` pane is left alone; `StagedUnstagedAndCommits` (`-r -w`) keeps its range and refreshes only the two working-tree rows. Ignored for PR and `--all-files`.
- **Render**: all files become one `Vec<Line>` sliced by `scroll_offset`. Syntax coloring happens at render, not parse: backends leave `highlighted_spans` as `None`; the diff view calls `App::color_visible_hunks` (`src/app/coloring.rs`) for hunks in the viewport ±1 screen (`colored_hunks` records visited hunks; rebuilding annotations clears it). Container grammars (`needs_full_file_highlight`: Vue, Svelte, PHP, ...) are colored from whole-file content at parse time (`enhance_with_full_file_highlight`); if that cannot run (file >1 MB, no content) they stay uncolored. Diff search highlighting patches `theme.search_match_bg` via `ui::text_utils::apply_search_highlight_*`. The comment navigator hides when no comment rows exist.
- **Persistence**: sessions live in `~/.local/share/tuicr/reviews/`; `active_sessions.json` beside `index.json` records open TUIs (pid, slug, path, last-seen). Saves use a storage lock plus temp-file rename with stale sidecar lock recovery. The TUI keeps a persisted-session snapshot so polling (`review_watch_interval_ms`), `:e` and autosave merge external comment adds/edits/deletes (`tuicr review ...`) without overwriting local edits. The session file exists as soon as a session is active; TUI-created empty ones are deleted on exit. `tuicr review delete|clearc|clear` delete a session they empty unless `active_sessions.json` lists a live owner. Local reloads re-run VCS diff loading and `.tuicrignore` filtering; full reloads persist file hashes and invalidated review markers without saving unrelated dirty state; strict commit subsets stay live-only; manual `:e` also reloads persisted comments before applying the diff. Successful comment submits autosave with a locked atomic write that merges external comments first. Switching local targets (`confirm_commit_selection`, `sessions_tab_select`) saves the outgoing session via `App::save_outgoing_session` in the loaders only, after the diff fetched (not on NoChanges/failed resumes), aborting on save failure; `sessions_tab_select` re-reads the selected JSON after the loader, since the row may be the open session. `clear_commit_pane` and `install_review_commits` move focus off a hidden commit pane.
- **Review CLI** (`src/review_cli.rs`, JSON only): `--repo` is a *selector*: a checkout path (local sessions plus PR sessions of its `origin`) or a forge coordinate (`owner/repo`, repo URL; matches by slug). A PR session resolves by its self-contained slug (`gh:owner/repo/pr/N`), no `--repo` needed. `review list` shows `active` and `kind`.
- **Comments**: `App::save_comment()` builds an `AddCommentRequest` for `add_comment_to_session()`; `:clip`/`:export` → `export_to_clipboard()` (stdout with `--stdout`).
- **PR review-requested toggle** (`r` on the Pull Requests tab, `ForgeKind::pr_list_scopes`): cycles all open → requesting me → (GitHub only) `PersonallyRequested`, i.e. `user-review-requested:@me`, excluding team requests. Bitbucket's `state="OPEN"` must live inside `q` (Cloud ignores a standalone `state` once `q` exists). Gerrit diverges: see `docs/FORGES.md` G4.

## Forge integration

Everything under `src/forge/` (GitHub `gh`, GitLab `glab`, Gitea `tea`, Bitbucket `bkt`, Azure DevOps, Gerrit). **Read `docs/FORGES.md` before** touching `src/forge/`, PR mode, session keys/lifecycle, PR submit, or any code that assigns `diff_files` in PR mode (commit-message pseudo-file, file registration, base/merge-base SHA, batched line counts, `e` in PR mode). It holds detection order, `ForgeBackend` contracts, async/session/submit rules and gotchas F1-F10, G1-G12.

## Hard-won gotchas

Numbered once. Forge/PR-mode gotchas (F*, G*) live in `docs/FORGES.md`.

1. **Patch display headers are never authoritative metadata.** Paths/statuses come from Git `--raw -z`, jj templates, Mercurial `status -0`, or forge JSON. `diff_parser` only consumes `@@` hunks, using their declared old/new counts as the grammar. Add no quoting or path-header heuristics.
2. **Anchor restore must scroll**: setting `diff_state.cursor_line` without `move_cursor_to_annotation` leaves the viewport at the top.
3. **`Comment`'s line anchor lives in the `HashMap` key, not `Comment.line_context`** (`Comment::new` leaves it unset). The submit mapper takes an explicit `CommentAnchor`; never infer "file-level" from `line_context.is_none()`.
4. **Comments are commit-scoped via `Comment::commit_id`.** With exactly one commit shown, `App::save_comment` stamps its SHA. `Some(sha)` comments are hidden when a different commit (or subset without `sha`) is selected; `None` (legacy, review-level, cumulative-diff) is always visible. `App::comment_visible(&Comment)` is the single predicate, applied in `rebuild_annotations`, both diff renderers, the navigator, and submit preflight. `AnnotatedLine::LineComment`/`FileComment` `comment_idx` is the **absolute** index into the stored value; `delete_comment_at_cursor` and `enter_edit_mode` must look it up directly, not re-count by side.
5. **Register diff files before `r`, `R`, or a comment can land on them.** All three look the file up in `ReviewSession.files` by display path (toggles return silently; `add_comment_to_session` errors `session does not contain file`). Any code assigning `self.diff_files` must call `App::register_diff_files` (narrowing the inline commit pane once skipped this).
6. **The `[TYPE]` prefix uses the config-resolved `label`**, not the `CommentType` id (`CommentType` has no label; `as_str()` is the uppercased id). `app::resolve_comment_type_label` is the single resolver; `App::comment_type_label` (TUI), the markdown export and `SubmitContext::type_label` (`src/forge/submit.rs`) delegate to it, and `describe_row` (`src/ui/submit_modals.rs`, which previews what gets posted) calls `App::comment_type_label`. Hence submit takes a `SubmitContext`, not a bare `&ForgeConfig`.
7. **Gates must run from the worktree**: `cd` into it before `cargo` (it resolves `Cargo.toml` from `pwd`; the wrong tree is exercised silently).
8. **TestBackend modal sizing**: 60%×40% on 120×24 is 72×9 (7 content rows). The submit confirmation modal needs 70% height, the submit-action picker 50% (footer line).
9. **GNU Linux release binaries stay dynamically linked** (static glibc can crash loading a host NSS module such as Fedora's `libnss_myhostname`); musl artifacts are the static builds. Direct updates must keep the running binary's GNU/musl target when choosing an asset.


28. **PR editor snapshots live in a private per-user cache dir keyed by the full revision SHA.** `editor_target::snapshot_root` (host, PR, the SHA actually fetched) sits under `ProjectDirs` cache `snapshots/` (`0700`, owner-checked); `materialize` creates directories without following symlinks and links an exclusively created temp file into place, so never write snapshots with `fs::write` or under the shared temp dir. `TUICR_GLAB_DEBUG` logs to `glab-debug.log` in the same cache dir (`0600`, `O_NOFOLLOW`). The directory name is a truncated readable slug plus a hash of host/repo/PR/SHA, so it stays under NAME_MAX for any namespace. The debug log refuses to append to an existing file that is not an owner-only regular file.

28. **Local slugs and `forge:` paths read `origin` and owners faithfully.** Gerrit's `remote_for_repository` reads remotes through `forge::remotes` (libgit2, then `git remote get-url`) so reftable/SHA-256 checkouts work. `resolve_owner_repo` (local slugs) still uses libgit2 only; changing it re-slugs existing reviews and must land with the storage identity change. `RepoCoordinate::parse` tries `forge:<host>/<owner…>/<repo>` (`parse_forge_path`) *before* the hostname parsers, which would strip Gerrit's `a/` and drop multi-segment owners.

### Keeping Docs Updated

When adding user-facing features, update the relevant documentation:

Document every configuration option in `docs/CONFIG.md`. Only critical
configuration options should also appear in `README.md`; document all other
configuration options in `docs/CONFIG.md` only.

| Document               | Update when adding/changing...                                                                                           |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `README.md`            | Keybindings, commands (`:*`), CLI flags, features list, installation methods, agent integration setup, forge limitations |
| `src/ui/help_popup.rs` | Keybindings or commands (update the `help_text` vector)                                                                  |
| `src/ui/status_bar.rs` | Keybindings worth advertising in the per-pane hint line                                                                  |
| `docs/CLI.md`          | Any clap flag, subcommand, env var, stderr marker, or exit code: add a row to the matching table                          |
| `docs/KEYBINDINGS.md`  | Any keybinding or `:` command: add a row to the pane's key table and the `:` command table, plus prose for new behavior  |
| `AGENTS.md`            | Key types, invariants, non-obvious behavior, general gotchas, repo-managed agent integrations (forge detail goes in `docs/FORGES.md`) |
| `docs/FORGES.md`       | Forge backend contracts, detection, session/submit rules, F*/G* gotchas (new forge-specific gotchas go here)             |
| `docs/CONFIG.md`       | Any `config.toml` key: add a row to the Options table and a line to the Full example block                               |

---

## Review Format (for coding agents receiving tuicr output)

When a user reviews your changes with tuicr, they will provide you with a structured Markdown document optimized for agent consumption.

### Document Structure

```markdown
I reviewed your code and have the following comments. Please address them.

Reviewing unstaged changes

Comment types: ISSUE (problems to fix), SUGGESTION (improvements), NOTE (observations), PRAISE (positive feedback)

Summary: {optional overall notes}

## Local tuicr Comments

1. **[ISSUE]** `src/auth.rs:42` - Magic number should be a named constant
2. **[SUGGESTION]** `src/api.rs` - Consider adding unit tests
3. **[NOTE]** `src/utils.rs:15` - Why was this approach chosen?
```

That is the default layout, not a fixed contract. The `[export]` section of `config.toml` lets a user reword or drop the intro line, the `Reviewing …` line, the comment-type legend, and either section heading. Parse the numbered comment list as the only part guaranteed to be present. See [docs/CONFIG.md](docs/CONFIG.md) for the keys.

In pull request mode the header block also includes `URL:` and `Head:` lines identifying the pull request and the commit reviewed, and unresolved upstream threads follow under their own heading.

### Comment Types

| Type             | Meaning                    | Action Required                 |
| ---------------- | -------------------------- | ------------------------------- |
| **[ISSUE]**      | Problem that must be fixed | Yes - address before proceeding |
| **[SUGGESTION]** | Improvement recommendation | Consider implementing           |
| **[NOTE]**       | Observation or question    | Respond or acknowledge          |
| **[PRAISE]**     | Positive feedback          | No action needed                |

### Comment Format

Each comment is numbered and self-contained:

- `{n}. **[TYPE]** \`{file}:{line}\` - {content}` (line comment)
- `{n}. **[TYPE]** \`{file}\` - {content}` (file comment)

You can reference comments by number (e.g., "Regarding comment #2...").

### How to Respond

#### 1. Address Issues First

**[ISSUE]** comments are blocking and must be fixed:

```markdown
## Issues Addressed

- #1 `src/auth.rs:42` - Extracted magic number to `TOKEN_EXPIRY_SECONDS` constant
```

#### 2. Consider Suggestions

**[SUGGESTION]** comments are non-blocking but should be considered. Either implement or explain why not.

#### 3. Answer Questions

**[NOTE]** comments often contain questions. Provide brief explanations.

### Response Format

```markdown
## Changes Made

### Issues Addressed

- #1 - Extracted magic number to `TOKEN_EXPIRY_SECONDS` constant

### Suggestions Implemented

- #2 - Added unit tests for API module

### Suggestions Declined

- #5 - Kept current approach because [reason]

### Notes Addressed

- #3 - This approach was chosen for performance reasons
```

## Tips for Coding Agents

1. **Read the full review** before making changes
2. **Prioritize [ISSUE] items** - these are blocking
3. **Reference comments by number** when responding
4. **Preserve reviewer intent** - don't over-engineer solutions
5. **Be concise** in your response
6. **If unclear**, ask for clarification rather than guessing
