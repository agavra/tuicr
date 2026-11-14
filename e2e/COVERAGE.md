# e2e coverage

Workflow area -> scenarios (`e2e/scenarios/<area>/<behaviour>.sh`; each file's header comment says what it checks and the PR/issue it covers). Run `e2e/run.sh --list` for the current set.

| Area | Workflow | Scenarios |
|---|---|---|
| `local` | Local reviews: git/hg/jj targets, commit selector, sessions tab, `-A`/`-p`/`--file --line`, `.tuicrignore`, editor handoff | 19: `all_files_lists_unchanged_files`, `binary_file_labelled`, `cli_backend_ignores_untracked_symlink`, `commit_range_forms`, `commit_range_to_staged_switch_resets_pane`, `commit_selector_cycle`, `editor_handoff_and_reload`, `external_git_add_survives_stage`, `file_line_start_annotate_export`, `hg_hunks_under_own_files`, `jj_commit_selector_and_comment`, `jj_revset_and_range`, `navigation_files_hunks_scroll`, `path_filter_limits_working_tree`, `sessions_tab_deletes_saved_review`, `sessions_tab_resumes_saved_review`, `smoke_open_and_quit`, `staged_unstaged_selection`, `tuicrignore_filters_files` |
| `tree` | File tree: filters, reviewed-file hiding, hit areas, horizontal scroll | 6: `file_picker_filter_jump`, `filter_include_exclude`, `hidden_list_not_clickable`, `hscroll_moves_whole_row`, `reviewed_files_hidden_burn_down`, `wrapped_line_rows_clickable` |
| `diff` | Diff view: wrap, side-by-side, scroll, goto/expand context, relative numbers, `:focus` | 8: `goto_line_and_expand_context`, `hscroll_side_by_side_by_display_width`, `relative_line_numbers_toggle`, `search`, `side_by_side_toggle`, `single_file_view_focus`, `wrap_default_off`, `wrap_toggle` |
| `comments` | Comment input, types, side-by-side anchors, `:clear*` | 7: `add_line_range_file_review`, `clear_commands`, `edit_and_delete`, `input_wraps_by_display_width`, `side_by_side_old_side_context_comment`, `summary_jump`, `types_cycle_label_and_export` |
| `review` | Reviewed marks, session persistence/merge, quit guards, diff watch | 8: `diff_watch_reloads_external_edit`, `external_session_edit_merged`, `marks_survive_narrow_and_widen`, `quit_guards_unsaved_comments`, `reload_and_restart_persist`, `reviewed_marks_and_progress`, `write_quit_offers_copy`, `zz_saves_zq_discards` |
| `theme` | `:theme` picker, apply/rollback, `--theme` | 5: `confirm_missing_file_rolls_back`, `picker_enter_applies_first_row`, `picker_preview_revert_keep`, `transparent_background_survives_switch`, `unknown_theme_exits_before_tui` |
| `input` | Prompts, mouse, paste, visual selection, goto guards | 4: `goto_line_in_collapsed_file_keeps_gap`, `paste_into_tree_filter_prompt`, `prompt_keeps_focus_on_mouse_click`, `visual_selection_across_files_rejected` |
| `help` | Help popup | 2: `open_search_close`, `scroll_clamped_on_resize` |
| `ui` | Chrome: header/chips, config layout, leader and quit keys | 2: `config_layout_leader_and_quit_key`, `header_and_chip_use_display_width` |
| `export` | Markdown export of local and remote threads | 2: `remote_thread_cr_stays_in_item`, `stdout_markdown` |
| `syntax` | Syntax colouring, user grammars | 2: `bad_user_grammar_skipped`, `vue_script_block_colored` |
| `cli` | Non-interactive CLI: `review` subcommands, flag validation, update detection, install script | 7: `conflicting_flags_rejected`, `install_script_without_tty`, `review_add_live`, `review_clear_keeps_session_open_in_tui`, `review_json_input_delete_clear`, `review_list_repo_selector_gerrit_path`, `update_detects_cargo_install_via_symlink` |
| `pr` | GitHub PR flows: open, reload, commit selector, threads, submit, editor snapshots, selector | 24: `commit_selector_narrow_cycle_widen`, `commits_command_reaches_local_checkout`, `description_panel`, `editor_snapshot_long_namespace`, `editor_snapshot_private_dir`, `forge_paths_special_chars_encoded`, `graphql_coordinates_sent_as_strings`, `head_move_resets_narrowed_selection`, `info_panel_laid_out_at_diff_width`, `late_reload_dropped_after_switching_pr`, `local_targets_from_subdirectory`, `open_rejected_while_head_keeps_moving`, `open_survives_head_move_once`, `reload_refreshes_cumulative_diff`, `remote_threads_visibility`, `selector_load_more_failure_keeps_rows`, `smoke_open_and_quit`, `submit_approve_and_request_changes`, `submit_comment_on_subset_only_file`, `submit_comment_posts_payload`, `submit_draft_pending_review`, `submit_old_side_range_maps_inline`, `submit_resolver_clips_tall_item`, `submit_resolver_scrolls_selection` |
| `gitlab` | GitLab MR: URL/host, threads, draft read-back, submit, partial submit | 5: `draft_notes_read_back`, `mr_url_host_normalised`, `partial_submit_receipts`, `submit_comment_approve_request_changes`, `threads_outdated_and_replied` |
| `gitea` | Gitea PR: pagination, votes, draft read-back, submit | 4: `draft_review_read_back`, `pr_info_effective_vote`, `pr_list_pagination_under_server_cap`, `submit_events_post_review` |
| `bitbucket` | Bitbucket Cloud: threads, submit | 2: `submit_comment_approve_request_changes`, `threads_file_level_and_deleted_root` |
| `gerrit` | Gerrit: URL, threads, root change (reads only) | 3: `browser_url_opens_change`, `root_change_diffs_against_empty_tree`, `thread_state_from_latest_reply` |
| `azure` | Azure DevOps: URL parsing and auth-error path only | 1: `pr_url_with_query_recognised` |

## Known gaps

* **Gerrit and Azure DevOps submit:** Gerrit refuses credentials over `http://`, so a fake would need HTTPS with a certificate tuicr trusts (platform verifier; not portable). Azure needs a fake for iterations/threads/PAT or `az rest`. Both forges' reads are covered where a fake exists.
* **Gitea range paths and Azure iteration anchors:** need a two-commit clone plus compare/fetch fakes; unit-tested only.
* **Not driven end to end yet:** Sessions tab `a` toggle, `:messages`/`:version`, mouse wheel/drag, the update install flow, `[export]` config keys, `scroll_offset`/`cursor_line`/`search_highlight`/`ignore_whitespace`/`initial_commit_selection`, comment-input editing keys and `comment_vim`, PR `:prs`/scope cycle/`:copy-url`.
* **Known failing, quarantined:** `diff/wrap_default_off` - `docs/CONFIG.md` documents `wrap = false`, a fresh tuicr wraps (see `quarantine.tsv`).
* **Environment-dependent:** `local/hg_hunks_under_own_files` needs real Mercurial and `local/jj_*` need Jujutsu; both SKIP without them (`E2E_REQUIRE_HG=1`/`E2E_REQUIRE_JJ=1` fail instead). `cli/install_script_without_tty` needs `install.sh` from the `site` branch.
