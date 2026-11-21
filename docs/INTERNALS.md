# App internals (tuicr)

Subsystem invariants, loaded on demand from `AGENTS.md`. Cross-cutting rules and the numbered general gotchas stay in `AGENTS.md`; forge and PR mode live in [FORGES.md](FORGES.md). User-facing behaviour is in [KEYBINDINGS.md](KEYBINDINGS.md), [CONFIG.md](CONFIG.md) and [CLI.md](CLI.md); this page holds only what the code needs beyond it.

## File tree filters (`FileTreeFilter`, `src/app/file_filter.rs`)

- `show_reviewed` (`:set reviewed!`, config `show_reviewed`) is command-only: single-stroke keys are reserved for vim motions.
- Two predicates on purpose: `file_matches_patterns()` (`i`/`e` only: the review *population*, used by `file_count()`, `reviewed_count()`, `/` search) vs `file_passes_filter()` (that plus `show_reviewed`). Counting visible rows would render the progress fraction as `0/n` when reviewed files are hidden. Only the file-level `reviewed` flag hides; a file with individually `R`-marked hunks stays visible. `is_hunk_reviewed` hashes every hunk, so call it only when the answer is needed.
- `FileTreeFilter` has a hand-written `Default` because `show_reviewed` defaults to *true*.
- Marking a file reviewed while hiding deletes the cursor's row, so `toggle_reviewed_for_file_idx` calls `advance_past_hidden_file()` (next visible file, wrapping, parking at the overview when empty) regardless of `adjust_cursor`.
- `file_filter.draft` makes the tree a text-input sub-state of `InputMode::Normal` (same shape as `pr_filter_draft`); `main.rs` routes to `map_file_tree_prompt_mode` while it is `Some`. An open prompt owns input whatever panel has focus (a click can move focus to the diff): `handler::handle_normal_action` checks it before `focused_panel`, and a diff drag is ignored while it is open (no `VisualSelect`).
- `handler::handle_paste` is the one paste table: prompts and picker filters accept paste; other Normal-mode paste is dropped on purpose.
- `main.rs` routes keys through `handler::map_normal_path_key` and `handler::dispatch_action`, so tests drive the real path. Keys are focus-scoped via `map_file_tree_mode`: the tree claims `i`/`e`/`I`/`E`/`/`; the diff keeps `i` = edit comment, `/` = search diff.
- Filters apply at render/measure time and are never persisted, unlike `.tuicrignore`, which drops files at load time.

## Modes and pickers (`InputMode`, `src/app/mod.rs`)

- `Summary`: `Enter` jumps to the comment in the continuous diff (leaving single-file view); a reviewed file/hunk is revealed without clearing its persisted state.
- `ThemePicker` (`src/app/theme_picker.rs`): previews go through `App::set_theme` (renderers read `&app.theme` per frame, so nothing else needs invalidating; `set_theme` also drops `App::pr_info_rows_cache`). `Enter` never writes `config.toml`; `Esc` reverts to a `Theme` snapshot taken on entry. `Theme` has a manual `Clone` because its `OnceLock` highlighter cache is not `Clone`. `:theme <name>` applies directly.
- `FilePicker` (`src/app/file_picker.rs`): built like `ThemePicker`. Candidates are captured on open from files passing `file_passes_filter`; a background reload can replace `diff_files` meanwhile, so `Enter` re-resolves by display path and re-checks the filter (`jump_to_file`).
- Comment navigation (`CommentNavigatorState`/`CommentNavigatorItem`) derives from `line_annotations`, so local comments and visible remote threads jump to the annotations the diff renders (`move_cursor_to_annotation`). The navigator hides when no comment rows exist.

## Startup, themes, syntaxes, update

- `src/syntax/mod.rs` translates the ANSI-slot placeholders some bat-compatible Base16 `.tmTheme` files use.
- `syntax::load_user_syntaxes` adds sibling `syntaxes/*.sublime-syntax` once, before the first `SyntaxHighlighter` exists; an absent/empty dir skips the ~200ms relink.
- `tuicr update` exits before TUI setup (flow in `src/update/`). `installation::detect_external_manager` (pacman DB ownership, Home Manager generation links under `$XDG_STATE_HOME`, declarative Nix profiles in `/etc/profiles/per-user` and `/run/current-system/sw`) runs before path-based detection. Homebrew core vs tap comes from the keg's `INSTALL_RECEIPT.json`, since the Cellar path is identical for both.
- **GNU Linux release binaries stay dynamically linked** (static glibc can crash loading a host NSS module such as Fedora's `libnss_myhostname`); musl artifacts are the static builds. Direct updates keep the running binary's GNU/musl target when choosing an asset.
- **`--line N`**: `App::pending_start_line` is applied by `apply_pending_start_line` after the first `terminal.draw`, because centering needs the viewport height render measures; it waits while a modal/selector is open or no rows exist. `main.rs` repaints in the same synchronized update so the unpositioned frame never shows.

## Render, coloring, layout

- All files become one `Vec<Line>` sliced by `scroll_offset`.
- Syntax coloring happens at render, not parse: backends leave `highlighted_spans` as `None`; the diff view calls `App::color_visible_hunks` (`src/app/coloring.rs`) for hunks in the viewport ±1 screen (`colored_hunks` records visited hunks; rebuilding annotations clears it). Container grammars (`needs_full_file_highlight`: Vue, Svelte, PHP, ...) are colored from whole-file content at parse time (`enhance_with_full_file_highlight`); if that cannot run (file >1 MB, no content) they stay uncolored.
- Diff search highlighting patches `theme.search_match_bg` via `ui::text_utils::apply_search_highlight_*`.
- With `file_list_position = "right"`, `App::focus_pane_left`/`focus_pane_right` mirror the `<leader>h`/`<leader>l` walk; mouse hit-testing reads the rendered `file_list_area`/`diff_area` rects, so it needs no change.
- **TestBackend modal sizing**: 60%×40% on 120×24 is 72×9 (7 content rows). The submit confirmation modal needs 70% height, the submit-action picker 50% (footer line).

## Diff watch and reloads

- A watch result of `NoChanges` is an empty diff, distinct from an unchanged fetch (`None`). Eligible staged and unstaged targets clear via `show_emptied_review`, preserving persisted comments. Empty-target checks use `narrowed_fetch_source`.
- **Staged/unstaged rows**: in Git the target selector and inline pane always show both rows, dimmed with "(no changes)" when a side is empty (`App::worktree_status`, updated on each diff-watch tick), so rows never shift. Opening an empty side, or `:e` on a review that emptied, shows an empty review with a message (`App::show_emptied_review` leaves the session unreconciled so marks and comments survive). The cursor starts on the first row with content (`App::initial_selector_cursor`). jj and Mercurial show a row only while its side has changes.
- `diff_watch_interval_ms` reloads leave a `DiffSource::CommitRange` pane alone; `StagedUnstagedAndCommits` (`-r -w`) keeps its range and refreshes only the two working-tree rows.
- Reloads of a `-r` range (watch ticks, `:e`) diff from the base resolved at startup (`App::revision_range`, used only while the fetched commit list matches), so `A...B` keeps its merge base.

## Persistence (`src/persistence/`, `src/review_store.rs`)

Local and PR session slug identity (`resolve_owner_repo`): `docs/FORGES.md` F12.

- Sessions live in the platform data dir's `reviews/` (`ProjectDirs`; `~/.local/share/tuicr/reviews/` on Linux); `active_sessions.json` beside `index.json` records open TUIs (pid, slug, path, last-seen); the registry replaces only the current pid's entry. PR session files and manifest buckets: FORGES.md G13.
- `ReviewStore` is the library facade; `add_comment_to_session()` is shared with `App::save_comment()` (which builds an `AddCommentRequest`).
- `update_session_in_dir` (`tuicr review add|delete|clear`) writes a local session back to the exact file it loaded (never a re-derived slug path) and indexes it by path only when it lives under `reviews/`.
- A corrupt `index.json` is moved to `index.json.corrupt` and rebuilt from `sessions/`, as is a missing `index.json` beside existing sessions (`manifest_for_update`/`read_manifest`).
- Session paths serialize through `path_codec` (non-UTF-8 Unix paths as a NUL-tagged hex string).
- Saves use a storage lock plus temp-file rename with stale sidecar lock recovery. Successful comment submits autosave with a locked atomic write that merges external comments first.
- The TUI keeps a persisted-session snapshot so polling (`review_watch_interval_ms`), `:e` and autosave merge external comment adds/edits/deletes (`tuicr review ...`) without overwriting local edits.
- The session file exists as soon as a session is active; TUI-created empty ones are deleted on exit. `tuicr review delete|clearc|clear` delete a session they empty unless `active_sessions.json` lists a live owner.
- `ReviewStore::delete_open_review` (Sessions tab `dd`) ignores only this process's own registry entry, so the review the TUI itself loaded is deletable; `App::forget_deleted_session` drops that in-memory copy, or autosave and diff reconciliation would write the deleted comments back.
- Local reloads re-run VCS diff loading and `.tuicrignore` filtering; full reloads persist file hashes and invalidated review markers without saving unrelated dirty state; strict commit subsets stay live-only; manual `:e` also reloads persisted comments before applying the diff.
- Switching local targets (`confirm_commit_selection`, `sessions_tab_select`) saves the outgoing session via `App::save_outgoing_session` in the loaders only, after the diff fetched (not on NoChanges/failed resumes), aborting on save failure; `sessions_tab_select` re-reads the selected JSON after the loader, since the row may be the open session.
- `clear_commit_pane` and `install_review_commits` move focus off a hidden commit pane.

## Clipboard and export (`src/output/markdown.rs`)

- `copy_method_order`: config `clipboard = [...]` replaces the per-environment order, with OSC 52 appended when omitted. Per-environment defaults: CLI.md "Environment variables"; on non-macOS, SSH (`SSH_TTY` or `SSH_CONNECTION`) and Rex use only OSC 52 (macOS keeps `pbcopy` first), since local tools would copy on the remote host.
- `clipboard_command` runs through `sh -c` (`cmd /C` on Windows) before all of them, in its own process group, with a fixed 5 s `CLIPBOARD_COMMAND_TIMEOUT` (the stdin write runs on a thread so a non-reading command cannot outlast it); success needs the whole text written and exit 0. On timeout the group is killed and the child reaped.
- Any failure falls back to that order and `ClipboardCopy::fallback_warning` names the failure and the method used; callers show it through `App::set_copy_status`, except the `:wq` copy offer, which quits at once and prints it to stderr after teardown (`App::pending_exit_warning`).
- `:clip`/`:export` → `export_to_clipboard()` (stdout with `--stdout`). `[export] code_context` blocks come from the loaded hunks (`App::export_diff()`), never fetched.
