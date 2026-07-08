use super::*;

impl App {
    pub fn new(
        theme: Theme,
        comment_type_configs: Option<Vec<CommentTypeConfig>>,
        output_to_stdout: bool,
        options: AppStartupOptions<'_>,
    ) -> Result<Self> {
        // `tuicr pr <target>` mode: enter PR review directly, skipping the
        // selector. Errors here surface before TUI startup like other
        // startup failures.
        if let Some(target) = options.pr_target {
            return Self::new_from_pr_target(
                theme,
                comment_type_configs,
                output_to_stdout,
                target,
                options.repo_url_override.clone(),
            );
        }

        // --file mode: open a single file for annotation without VCS
        if let Some(file_path) = options.file_path {
            let vcs = Box::new(FileBackend::new(file_path)?);
            let vcs_info = vcs.info().clone();
            let highlighter = theme.syntax_highlighter();
            let diff_files = vcs.get_working_tree_diff(highlighter)?;
            let session = Self::load_or_create_session(&vcs_info, SessionDiffSource::WorkingTree);

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs,
                output_to_stdout,
                diff_files,
                session,
                DiffSource::WorkingTree,
                InputMode::Normal,
                Vec::new(),
                None, // no path_filter
                options.repo_url_override.clone(),
            )?;

            // Hide the file list only when reviewing a single file; in
            // directory mode the user needs the list to navigate.
            if app.diff_files.len() == 1 {
                app.show_file_list = false;
            }
            app.focused_panel = FocusedPanel::Diff;

            return Ok(app);
        }

        // --all-files mode: enumerate every tracked file via `git ls-files`
        // and render in context-only mode for whole-repo annotation. Git-only
        // for MVP; non-git invocation surfaces as `NotARepository`.
        if options.all_files {
            let cwd = std::env::current_dir()
                .map_err(|_| TuicrError::NotARepository)?
                .canonicalize()
                .map_err(|_| TuicrError::NotARepository)?;
            let paths = crate::vcs::pristine::collect_tracked_paths(&cwd)?;

            let mut joined = Vec::new();
            for path in &paths {
                joined.extend_from_slice(path.as_os_str().as_encoded_bytes());
                joined.push(b'\n');
            }
            let path_hash = crate::hash::fnv1a_64(&joined);
            let head_or_none = crate::vcs::pristine::head_short_sha(&cwd);
            let base_commit = format!("pristine:{head_or_none}:{path_hash:016x}");

            let vcs = Box::new(FileBackend::new_pristine(paths, cwd.clone())?);
            let mut vcs_info = vcs.info().clone();
            vcs_info.head_commit = base_commit;
            let highlighter = theme.syntax_highlighter();
            let diff_files = vcs.get_working_tree_diff(highlighter)?;
            // `git ls-files` already honors `.gitignore`, but `.tuicrignore`
            // is tuicr-specific and not known to git. Run the same post-VCS
            // filter every other mode uses so users can elide tracked-but-
            // boring files (lockfiles, generated docs) from the review surface.
            let diff_files = Self::filter_ignored_diff_files(&cwd, diff_files);
            if diff_files.is_empty() {
                return Err(TuicrError::NoChanges);
            }
            let session = Self::load_or_create_session(&vcs_info, SessionDiffSource::Pristine);

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs,
                output_to_stdout,
                diff_files,
                session,
                DiffSource::WorkingTree,
                InputMode::Normal,
                Vec::new(),
                None, // no path_filter
                options.repo_url_override.clone(),
            )?;

            app.is_pristine_mode = true;
            app.focused_panel = FocusedPanel::Diff;
            // Force unified view: pristine mode has no diff, so side-by-side
            // would render two identical panes. The `:diff` command is gated
            // separately so the user cannot toggle back.
            app.diff_view_mode = DiffViewMode::Unified;
            // Default `--all-files` to single-file view: every tracked file
            // in one continuous scroll is overwhelming on large repos -- both
            // visually and at startup, since pristine still loads the whole
            // tree before render. `:focus` / `<leader>f` toggles back.
            app.is_single_file_view = true;
            // Snap the viewport to the first file's start.
            let start = app.calculate_file_scroll_offset(app.diff_state.current_file_idx);
            app.diff_state.scroll_offset = start;
            app.diff_state.cursor_line = start;

            return Ok(app);
        }

        let vcs = crate::profile::time("startup.detect_vcs", || {
            detect_vcs(options.git_backend_preference, options.diff_whitespace_mode)
        })?;
        let vcs_info = vcs.info().clone();
        let highlighter =
            crate::profile::time("startup.syntax_highlighter", || theme.syntax_highlighter());
        // Determine the diff source, files, and session based on input.
        // Four paths:
        //   1. -r + -w: combined commit range and uncommitted changes
        //   2. -r only: commit range
        //   3. -w only: working tree directly (skip commit selector)
        //   4. neither: commit selection UI
        if let Some(revisions) = options.revisions {
            let revision_range = crate::profile::time_with(
                "startup.resolve_revision_range",
                || vcs.resolve_revision_range(revisions),
                |result| match result {
                    Ok(range) => format!("commits={}", range.commit_ids.len()),
                    Err(e) => format!("error={e}"),
                },
            )?;
            let commit_ids = revision_range.commit_ids.to_vec();

            if options.working_tree {
                // Combined: commit range + staged/unstaged changes
                let diff_files = Self::get_working_tree_with_commits_diff_with_ignore(
                    vcs.as_ref(),
                    &vcs_info.root_path,
                    &commit_ids,
                    highlighter,
                    options.path_filter,
                )?;
                let session = Self::load_or_create_staged_unstaged_and_commits_session(
                    &vcs_info,
                    &commit_ids,
                );
                let review_commits: Vec<CommitInfo> = crate::profile::time_with(
                    "startup.selected_commit_info",
                    || vcs.get_commits_info(&commit_ids),
                    profile_commit_result,
                )?
                .into_iter()
                .rev()
                .collect();
                // Prepend staged/unstaged entries only when the backend supports them
                let change_status = Self::get_change_status_with_ignore(
                    vcs.as_ref(),
                    &vcs_info.root_path,
                    highlighter,
                    options.path_filter,
                )?;
                let mut all_commits = Vec::new();
                if change_status.staged {
                    all_commits.push(Self::staged_commit_entry());
                }
                if change_status.unstaged {
                    all_commits.push(Self::unstaged_commit_entry());
                }
                all_commits.extend(review_commits);

                let mut app = Self::build(
                    vcs,
                    vcs_info,
                    theme,
                    comment_type_configs.clone(),
                    output_to_stdout,
                    diff_files,
                    session,
                    DiffSource::StagedUnstagedAndCommits(commit_ids),
                    InputMode::Normal,
                    Vec::new(),
                    options.path_filter,
                    options.repo_url_override.clone(),
                )?;

                app.range_diff_files = Some(app.diff_files.clone());
                app.commit_list = all_commits.clone();
                app.commit_list_cursor = 0;
                app.commit_selection_range = if all_commits.is_empty() {
                    None
                } else {
                    Some((0, all_commits.len() - 1))
                };
                app.commit_list_scroll_offset = 0;
                app.visible_commit_count = all_commits.len();
                app.has_more_commit = false;
                app.show_commit_selector = all_commits.len() > 1;
                app.commit_diff_cache.clear();
                app.review_commits = all_commits;
                app.insert_commit_message_if_single();
                app.sort_files_by_directory(true);
                app.expand_all_dirs();
                app.rebuild_annotations();

                return Ok(app);
            }

            // Resolve the revisions to commits and diff as a commit range
            let diff_files = Self::get_commit_range_diff_with_ignore(
                vcs.as_ref(),
                &vcs_info.root_path,
                &revision_range,
                highlighter,
                options.path_filter,
            )?;
            let session = Self::load_or_create_commit_range_session(&vcs_info, &commit_ids);
            // Get commit info for the inline commit selector
            let review_commits = crate::profile::time_with(
                "startup.selected_commit_info",
                || vcs.get_commits_info(&commit_ids),
                profile_commit_result,
            )?;
            // Reverse to newest-first display order
            let review_commits: Vec<CommitInfo> = review_commits.into_iter().rev().collect();

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs.clone(),
                output_to_stdout,
                diff_files,
                session,
                DiffSource::CommitRange(commit_ids),
                InputMode::Normal,
                Vec::new(),
                options.path_filter,
                options.repo_url_override.clone(),
            )?;

            // Set up inline commit selector for multi-commit reviews
            if review_commits.len() > 1 {
                app.range_diff_files = Some(app.diff_files.clone());
                app.commit_list = review_commits.clone();
                app.commit_list_cursor = 0;
                app.commit_selection_range = Some((0, review_commits.len() - 1));
                app.commit_list_scroll_offset = 0;
                app.visible_commit_count = review_commits.len();
                app.has_more_commit = false;
                app.show_commit_selector = true;
                app.commit_diff_cache.clear();
            }
            app.review_commits = review_commits;
            app.insert_commit_message_if_single();
            app.sort_files_by_directory(true);
            app.expand_all_dirs();
            app.rebuild_annotations();

            Ok(app)
        } else if options.working_tree {
            // Skip commit selector, go straight to working tree diff
            let diff_files = Self::get_working_tree_diff_with_ignore(
                vcs.as_ref(),
                &vcs_info.root_path,
                highlighter,
                options.path_filter,
            )?;
            let session =
                Self::load_or_create_session(&vcs_info, SessionDiffSource::StagedAndUnstaged);

            let app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs,
                output_to_stdout,
                diff_files,
                session,
                DiffSource::StagedAndUnstaged,
                InputMode::Normal,
                Vec::new(),
                options.path_filter,
                options.repo_url_override.clone(),
            )?;

            Ok(app)
        } else {
            let change_status = Self::get_change_status_with_ignore(
                vcs.as_ref(),
                &vcs_info.root_path,
                highlighter,
                options.path_filter,
            )?;
            let has_staged_changes = change_status.staged;
            let has_unstaged_changes = change_status.unstaged;

            // No eager working-tree-diff fetch — the selector only needs to
            // know whether to render the Staged/Unstaged rows. The actual
            // diff loads when the user picks one (load_staged_selection /
            // load_unstaged_selection / load_staged_and_unstaged_selection).

            let commits = crate::profile::time_with(
                "startup.recent_commits",
                || vcs.get_recent_commits(0, VISIBLE_COMMIT_COUNT),
                profile_commit_result,
            )?;
            if !has_staged_changes && !has_unstaged_changes && commits.is_empty() {
                return Err(TuicrError::NoChanges);
            }

            let mut commit_list = commits.clone();
            if has_staged_changes {
                commit_list.insert(0, Self::staged_commit_entry());
            }
            if has_unstaged_changes {
                commit_list.insert(0, Self::unstaged_commit_entry());
            }

            let diff_source = if has_staged_changes && has_unstaged_changes {
                DiffSource::StagedAndUnstaged
            } else if has_staged_changes {
                DiffSource::Staged
            } else if has_unstaged_changes {
                DiffSource::Unstaged
            } else {
                DiffSource::WorkingTree
            };

            let session_source = if has_staged_changes && has_unstaged_changes {
                SessionDiffSource::StagedAndUnstaged
            } else if has_staged_changes {
                SessionDiffSource::Staged
            } else if has_unstaged_changes {
                SessionDiffSource::Unstaged
            } else {
                SessionDiffSource::WorkingTree
            };

            let session = Self::load_or_create_session(&vcs_info, session_source);

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                comment_type_configs,
                output_to_stdout,
                Vec::new(),
                session,
                diff_source,
                InputMode::CommitSelect,
                commit_list,
                options.path_filter,
                options.repo_url_override.clone(),
            )?;

            app.has_more_commit = commits.len() >= VISIBLE_COMMIT_COUNT;
            app.visible_commit_count = app.commit_list.len();
            Ok(app)
        }
    }

    /// Shared constructor: all `App::new` paths converge here.
    ///
    /// `pub(crate)` so render-snapshot tests in `ui::app_layout` can drive
    /// the full app through `render` without going through `App::new`'s
    /// filesystem/VCS requirements.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build(
        vcs: Box<dyn VcsBackend>,
        vcs_info: VcsInfo,
        theme: Theme,
        comment_type_configs: Option<Vec<CommentTypeConfig>>,
        output_to_stdout: bool,
        diff_files: Vec<DiffFile>,
        mut session: ReviewSession,
        diff_source: DiffSource,
        input_mode: InputMode,
        commit_list: Vec<CommitInfo>,
        path_filter: Option<&str>,
        repo_url_override: Option<ForgeRepository>,
    ) -> Result<Self> {
        // Ensure all diff files are registered in the session. Persisted PR
        // subsets hydrate through the full PR diff first; keep subset-specific
        // hunk keys alive until the selected diff is loaded.
        let preserve_hunks = matches!(diff_source, DiffSource::PullRequest(_))
            && session.commit_selection_range.is_some();
        Self::register_diff_files(&mut session, &diff_files, preserve_hunks);

        let has_more_commit = commit_list.len() >= VISIBLE_COMMIT_COUNT;
        let visible_commit_count = if commit_list.is_empty() {
            VISIBLE_COMMIT_COUNT
        } else {
            commit_list.len()
        };

        let comment_types = Self::resolve_comment_types(&theme, comment_type_configs);
        let default_comment_type = Self::first_comment_type(&comment_types);
        let session_path = crate::persistence::storage::session_path(&session).ok();
        let session_file_state = session_path
            .as_deref()
            .filter(|path| path.exists())
            .and_then(|path| SessionFileState::from_path(path).ok());
        let persisted_session_snapshot = session.clone();

        let mut app = Self {
            theme,
            vcs,
            vcs_info,
            session,
            persisted_session_snapshot,
            session_path,
            session_file_state,
            review_watch_interval: Some(Duration::from_millis(DEFAULT_REVIEW_WATCH_INTERVAL_MS)),
            next_review_watch_at: Instant::now()
                + Duration::from_millis(DEFAULT_REVIEW_WATCH_INTERVAL_MS),
            ephemeral_session_paths: HashSet::new(),
            diff_files,
            diff_source,
            pending_editor_target: None,
            input_mode,
            focused_panel: FocusedPanel::Diff,
            diff_view_mode: DiffViewMode::Unified,
            file_list_state: FileListState::default(),
            comment_navigator_state: CommentNavigatorState::default(),
            diff_state: DiffState::default(),
            help_state: HelpState::default(),
            command_buffer: String::new(),
            command_completion: None,
            search_buffer: String::new(),
            last_search_pattern: None,
            comment_buffer: String::new(),
            comment_cursor: 0,
            comment_vim_enabled: false,
            comment_tab_width: 4,
            comment_vim_editor: None,
            comment_vim_command: None,
            comment_vim_pending: CommentVimPending::None,
            comment_type: default_comment_type,
            comment_types,
            comment_is_review_level: false,
            comment_is_file_level: true,
            comment_line: None,
            editing_comment_id: None,
            visual_selection: None,
            mouse_drag_active: false,
            comment_line_range: None,
            commit_list,
            commit_list_cursor: 0,
            commit_list_scroll_offset: 0,
            commit_list_viewport_height: 0,
            commit_selection_range: None,
            visible_commit_count,
            commit_page_size: COMMIT_PAGE_SIZE,
            has_more_commit,
            target_tab: TargetTab::Local,
            forge_repository: None,
            repo_url_override,
            canonical_resolved: false,
            pr_tab: PullRequestsTab::new(None),
            pr_list_viewport_height: 0,
            pr_list_inner_area: None,
            pr_filter_draft: None,
            pr_load_rx: None,
            pr_open_state: None,
            pr_open_rx: None,
            pr_reload_state: None,
            pr_reload_rx: None,
            forge_backend: None,
            forge_review_threads: Vec::new(),
            forge_review_summaries: Vec::new(),
            forge_review_threads_loading: false,
            pr_threads_rx: None,
            forge_config: crate::config::ForgeConfig::default(),
            username: crate::model::comment::DEFAULT_AUTHOR.to_string(),
            submit_state: None,
            submit_picker_cursor: 0,
            pr_submit_state: None,
            pr_submit_rx: None,
            current_pr_head: None,
            should_quit: false,
            dirty: false,
            quit_warned: false,
            message: None,
            pending_confirm: None,
            supports_keyboard_enhancement: false,
            show_file_list: true,
            is_pristine_mode: false,
            is_single_file_view: false,
            primed_walk_next: false,
            primed_walk_prev: false,
            down_released_since_arm: false,
            up_released_since_arm: false,
            cursor_line_highlight: true,
            leader_key: crate::config::DEFAULT_LEADER_KEY,
            scroll_offset: 0,
            file_list_area: None,
            comment_navigator_area: None,
            diff_area: None,
            file_list_inner_area: None,
            comment_navigator_inner_area: None,
            diff_inner_area: None,
            commit_list_inner_area: None,
            diff_row_to_annotation: Vec::new(),
            expanded_dirs: HashSet::new(),
            expanded_top: HashMap::new(),
            expanded_bottom: HashMap::new(),
            file_line_count_cache: HashMap::new(),
            line_annotations: Vec::new(),
            output_to_stdout,
            pending_stdout_output: None,
            comment_cursor_screen_pos: None,
            comment_input_annotation_offset: None,
            update_info: None,
            pending_count: None,
            review_commits: Vec::new(),
            pr_commits: Vec::new(),
            pr_last_reviewed_commit_index: None,
            pr_range_reload_state: None,
            pr_range_reload_rx: None,
            show_commit_selector: false,
            commit_diff_cache: HashMap::new(),
            range_diff_files: None,
            saved_inline_selection: None,
            path_filter: path_filter.map(|s| s.to_string()),
            export_legend: true,
        };
        // Auto-hide file list when path filter matches exactly one file
        if app.path_filter.is_some() && app.diff_files.len() == 1 {
            app.show_file_list = false;
            app.focused_panel = FocusedPanel::Diff;
        }
        app.sort_files_by_directory(true);
        app.expand_all_dirs();
        app.populate_file_line_count_cache();
        app.rebuild_annotations();
        app.detect_forge_repository();
        Ok(app)
    }

    /// Detect a GitHub forge repository from the local checkout, if any.
    /// Lazily called during startup — running this synchronously is fine
    /// because it only reads local config, never the network.
    fn detect_forge_repository(&mut self) {
        // `--repo-url` short-circuits detection: the user has told us
        // exactly which repo to target, so skip both the local-remote
        // probe and the `gh api` parent lookup that runs on PR-tab entry.
        if let Some(override_repo) = self.repo_url_override.clone() {
            self.forge_repository = Some(override_repo.clone());
            self.pr_tab = PullRequestsTab::new(Some(override_repo));
            self.canonical_resolved = true;
            return;
        }
        let repo = crate::forge::detect_forge_repository(&self.vcs_info.root_path);
        self.forge_repository = repo.clone();
        self.pr_tab = PullRequestsTab::new(repo);
    }

    fn resolve_comment_types(
        theme: &Theme,
        comment_type_configs: Option<Vec<CommentTypeConfig>>,
    ) -> Vec<CommentTypeDefinition> {
        let defaults = vec![
            CommentTypeDefinition {
                id: "note".to_string(),
                label: "note".to_string(),
                definition: Some("observations".to_string()),
                color: Some(theme.comment_note),
            },
            CommentTypeDefinition {
                id: "suggestion".to_string(),
                label: "suggestion".to_string(),
                definition: Some("improvements".to_string()),
                color: Some(theme.comment_suggestion),
            },
            CommentTypeDefinition {
                id: "issue".to_string(),
                label: "issue".to_string(),
                definition: Some("problems to fix".to_string()),
                color: Some(theme.comment_issue),
            },
            CommentTypeDefinition {
                id: "praise".to_string(),
                label: "praise".to_string(),
                definition: Some("positive feedback".to_string()),
                color: Some(theme.comment_praise),
            },
        ];

        let Some(configs) = comment_type_configs else {
            return defaults;
        };

        let mut resolved = Vec::new();
        for config in configs {
            let id = config.id;
            let label = config.label.unwrap_or_else(|| id.clone());
            let definition = config.definition;
            let color = config.color.as_deref().and_then(Self::parse_config_color);
            resolved.push(CommentTypeDefinition {
                id,
                label,
                definition,
                color,
            });
        }

        if resolved.is_empty() {
            defaults
        } else {
            resolved
        }
    }

    fn first_comment_type(comment_types: &[CommentTypeDefinition]) -> CommentType {
        comment_types
            .first()
            .map(|comment_type| CommentType::from_id(&comment_type.id))
            .unwrap_or_default()
    }

    pub(in crate::app) fn default_comment_type(&self) -> CommentType {
        Self::first_comment_type(&self.comment_types)
    }

    fn parse_config_color(value: &str) -> Option<Color> {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return None;
        }

        if let Some(hex) = normalized.strip_prefix('#')
            && hex.len() == 6
            && let Ok(rgb) = u32::from_str_radix(hex, 16)
        {
            let r = ((rgb >> 16) & 0xff) as u8;
            let g = ((rgb >> 8) & 0xff) as u8;
            let b = (rgb & 0xff) as u8;
            return Some(Color::Rgb(r, g, b));
        }

        match normalized.as_str() {
            "black" => Some(Color::Black),
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "yellow" => Some(Color::Yellow),
            "blue" => Some(Color::Blue),
            "magenta" => Some(Color::Magenta),
            "cyan" => Some(Color::Cyan),
            "gray" | "grey" => Some(Color::Gray),
            "darkgray" | "dark_gray" | "darkgrey" | "dark_grey" => Some(Color::DarkGray),
            "lightred" | "light_red" => Some(Color::LightRed),
            "lightgreen" | "light_green" => Some(Color::LightGreen),
            "lightyellow" | "light_yellow" => Some(Color::LightYellow),
            "lightblue" | "light_blue" => Some(Color::LightBlue),
            "lightmagenta" | "light_magenta" => Some(Color::LightMagenta),
            "lightcyan" | "light_cyan" => Some(Color::LightCyan),
            "white" => Some(Color::White),
            _ => None,
        }
    }

    pub fn comment_type_label(&self, comment_type: &CommentType) -> String {
        if let Some(definition) = self
            .comment_types
            .iter()
            .find(|definition| definition.id == comment_type.id())
        {
            return definition.label.to_ascii_uppercase();
        }

        comment_type.as_str()
    }

    pub fn comment_type_color(&self, comment_type: &CommentType) -> Color {
        if let Some(definition) = self
            .comment_types
            .iter()
            .find(|definition| definition.id == comment_type.id())
            && let Some(color) = definition.color
        {
            return color;
        }

        match comment_type.id() {
            "note" => self.theme.comment_note,
            "suggestion" => self.theme.comment_suggestion,
            "issue" => self.theme.comment_issue,
            "praise" => self.theme.comment_praise,
            _ => self.theme.fg_secondary,
        }
    }

    pub(in crate::app) fn register_diff_files(
        session: &mut ReviewSession,
        diff_files: &[DiffFile],
        preserve_hunks: bool,
    ) {
        for file in diff_files {
            if preserve_hunks {
                session.add_diff_file_preserving_hunks(file);
            } else {
                session.add_diff_file(file);
            }
        }
    }

    /// Direct-entry PR open: `tuicr pr <target>`.
    pub fn new_from_pr_target(
        theme: Theme,
        comment_type_configs: Option<Vec<CommentTypeConfig>>,
        output_to_stdout: bool,
        target: &str,
        repo_url_override: Option<ForgeRepository>,
    ) -> Result<Self> {
        use crate::forge::github::gh::parse_pull_request_target;
        use crate::forge::gitlab::glab::parse_pull_request_target_gitlab;
        use crate::forge::pr_open::open_pull_request;
        use crate::forge::traits::ForgeKind;

        // Try GitHub-style target first (numeric, GitHub URL, owner/repo#N).
        // If it embeds a GitLab URL, the GitLab parser picks it up.
        let parsed = parse_pull_request_target(target)
            .or_else(|_| parse_pull_request_target_gitlab(target))?;

        // Resolution order when the target lacks an explicit repo
        // (`tuicr pr 125`):
        //   1. `--repo-url` override (explicit user intent; no I/O)
        //   2. canonical of the local `origin` (gh api parent lookup —
        //      so `tuicr pr 125` from a fork checkout opens the PR on
        //      the upstream, matching the PR-tab behavior)
        //   3. detected local `origin` as the final fallback
        // URL- and owner-repo-hash targets carry their own repository, which
        // wins over all of the above since it's PR-specific.
        let local_repo_root = std::env::current_dir().ok();
        let detected_repo = local_repo_root
            .as_deref()
            .and_then(crate::forge::detect_forge_repository);

        // Canonical resolution (fork parent lookup) only works for GitHub.
        let canonical_repo = detected_repo.as_ref().and_then(|origin| {
            if origin.kind == ForgeKind::GitHub {
                use crate::forge::canonical::resolve_canonical_repository;
                use crate::forge::github::gh::SystemGhRunner;
                Some(resolve_canonical_repository(
                    origin,
                    repo_url_override.as_ref(),
                    &SystemGhRunner,
                ))
            } else {
                None
            }
        });
        let target_repo = parsed
            .repository
            .clone()
            .or_else(|| repo_url_override.clone())
            .or_else(|| canonical_repo.clone())
            .or_else(|| detected_repo.clone())
            .ok_or_else(|| {
                TuicrError::Forge(
                    "tuicr pr <number> requires a local forge remote. \
                     Use owner/repo#N or a full PR URL outside a checkout."
                        .to_string(),
                )
            })?;

        // Use the local checkout for `.tuicrignore` only when it matches the
        // PR's target repository — using a foreign repo's checkout would
        // mis-filter the PR diff.
        let local_checkout_for_target = local_repo_root
            .as_deref()
            .and_then(|root| crate::forge::local_checkout_for_repo(root, &target_repo));

        let backend = create_forge_backend(&target_repo, local_checkout_for_target.clone());
        let highlighter = theme.syntax_highlighter();
        let opened = open_pull_request(
            backend.as_ref(),
            parsed,
            local_checkout_for_target.as_deref(),
            highlighter,
        )?;
        let opened = Self::opened_pr_with_persisted_session(opened)?;

        let pr_source = PullRequestDiffSource::from_details(&opened.details);
        let diff_source = DiffSource::PullRequest(Box::new(pr_source));
        let vcs_info = VcsInfo {
            root_path: opened.session.repo_path.clone(),
            head_commit: opened.details.head_sha.clone(),
            branch_name: Some(opened.details.head_ref_name.clone()),
            vcs_type: VcsType::File,
        };
        // FileBackend acts as a no-op VCS placeholder; PR context expansion
        // routes through the forge backend, not the VCS box.
        let vcs: Box<dyn VcsBackend> = Box::new(PrNoopVcs::new(vcs_info.clone()));

        // Snapshot the PR details before consuming `opened` so we can kick
        // off the remote-thread fetch after `Self::build` returns.
        let details_for_threads = opened.details.clone();
        let commits_for_selector = opened.commits.clone();
        let review_metadata = opened.review_metadata.clone();
        let mut app = Self::build(
            vcs,
            vcs_info,
            theme,
            comment_type_configs,
            output_to_stdout,
            opened.diff_files,
            opened.session,
            diff_source,
            InputMode::Normal,
            Vec::new(),
            None,
            repo_url_override,
        )?;

        // Wire the forge backend so context expansion routes through it.
        app.forge_backend = Some(backend);
        app.forge_repository = Some(target_repo);
        // PR open establishes the target repo directly; no further canonical
        // resolution needed on PR-tab entry (which won't happen anyway since
        // the user came straight from CLI into PR diff mode).
        app.canonical_resolved = true;
        app.current_pr_head = Some(details_for_threads.head_sha.clone());
        let since_last_review_message =
            app.apply_pr_commit_selector(commits_for_selector, review_metadata);
        if matches!(&app.diff_source, DiffSource::PullRequest(_))
            && let Some(range) = app.commit_selection_range
            && !app.pr_commits.is_empty()
            && (range.0 > 0 || range.1 + 1 < app.pr_commits.len())
        {
            app.spawn_pr_range_reload();
        }
        if let DiffSource::PullRequest(pr) = &app.diff_source.clone()
            && pr.is_read_only()
        {
            let reason = pr.read_only_reason().unwrap_or("read only");
            app.set_warning(format!("This PR is {reason} — review is read-only"));
        } else if let Some(message) = since_last_review_message {
            app.set_message(message);
        }
        // Spawn thread-fetch on startup; the main event loop will drain
        // the receiver via `poll_pr_threads_events` once it begins.
        app.spawn_pr_threads_fetch(&details_for_threads, local_checkout_for_target);
        Ok(app)
    }
}
