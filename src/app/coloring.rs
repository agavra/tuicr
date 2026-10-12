use super::*;
use crate::syntax::SyntaxHighlighter;
use crate::vcs::{apply_new_side_prefix_highlight, file_needs_full_file_highlight};

impl App {
    /// Color uncolored visible hunks. Call this before building a line.
    ///
    /// Returns how many hunks it visited.
    ///
    /// `max_rows` is the frame's height.
    /// `diff_state.viewport_height` is written during render.
    pub(crate) fn color_visible_hunks(&mut self, max_rows: usize) -> usize {
        let highlighter = self.theme.syntax_highlighter_arc();
        let unvisited: Vec<(usize, usize)> = self
            .visible_hunk_indices(max_rows)
            .into_iter()
            .filter(|key| self.colored_hunks.insert(*key))
            .collect();

        unvisited
            .into_iter()
            .filter_map(|(file_idx, hunk_idx)| {
                let file = self.diff_files.get(file_idx)?;
                let hunk = file.hunks.get(hunk_idx)?;

                // only color uncolored lines
                if hunk.lines.iter().any(|l| l.highlighted_spans.is_some()) {
                    return Some(());
                }
                let path = file.display_path().clone();
                if file_needs_full_file_highlight(file, &path)
                    && self.full_file_attempted.insert(file_idx)
                    && self.color_file_with_context(file_idx, &path, &highlighter)
                {
                    return Some(());
                }
                let hunk = self.diff_files.get_mut(file_idx)?.hunks.get_mut(hunk_idx)?;
                highlighter.rehighlight_hunk_in_place(hunk, &path);
                Some(())
            })
            .count()
    }

    /// Color every hunk of a file whose hunks need the lines before them
    /// (e.g. Python docstrings), using the file content from the context
    /// provider. Each hunk is first colored on its own, so deletion lines and
    /// a failed highlight still get colors, then new-side lines are overlaid
    /// with spans from the file prefix.
    ///
    /// Returns false, coloring nothing, when the content could not be fetched.
    fn color_file_with_context(
        &mut self,
        file_idx: usize,
        path: &Path,
        highlighter: &SyntaxHighlighter,
    ) -> bool {
        let file = &self.diff_files[file_idx];
        let Some(last_line) = file
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter_map(|l| l.new_lineno)
            .max()
        else {
            return false;
        };
        let fetched = self.context_provider().fetch_context_lines(
            file.old_path.as_ref(),
            file.new_path.as_ref(),
            file.status,
            1,
            last_line,
        );
        let prefix: Vec<String> = match fetched {
            Ok(lines) if lines.first().and_then(|l| l.new_lineno) == Some(1) => {
                lines.into_iter().map(|l| l.content).collect()
            }
            _ => return false,
        };

        let file = &mut self.diff_files[file_idx];
        for hunk in &mut file.hunks {
            if hunk.lines.iter().all(|l| l.highlighted_spans.is_none()) {
                highlighter.rehighlight_hunk_in_place(hunk, path);
            }
        }
        apply_new_side_prefix_highlight(file, highlighter, path, &prefix);
        true
    }

    /// Visible hunks are those in the viewport plus and minus one screen
    pub(crate) fn visible_hunk_indices(&self, max_rows: usize) -> Vec<(usize, usize)> {
        let height = max_rows.max(1);
        let len = self.line_annotations.len();
        let offset = self.diff_state.scroll_offset.min(len);

        // Calculate one screen of margin above and below the viewport to color
        let start = offset.saturating_sub(height);
        let end = offset.saturating_add(height * 2).min(len);

        let mut hunks: Vec<(usize, usize)> = self.line_annotations[start..end]
            .iter()
            .filter_map(AnnotatedLine::hunk_ref)
            .collect();
        // A hunk's rows are contiguous once non-hunk rows are filtered out, so
        // dropping adjacent repeats leaves each hunk once.
        hunks.dedup();
        hunks
    }
}

impl AnnotatedLine {
    fn hunk_ref(&self) -> Option<(usize, usize)> {
        match self {
            Self::DiffLine {
                file_idx, hunk_idx, ..
            }
            | Self::SideBySideLine {
                file_idx, hunk_idx, ..
            }
            | Self::HunkHeader { file_idx, hunk_idx } => Some((*file_idx, *hunk_idx)),
            // Exhaustive list to force new variants to be considered.
            Self::PrInfoLine { .. }
            | Self::IssueCommentsHeader
            | Self::IssueComment { .. }
            | Self::ReviewCommentsHeader
            | Self::ReviewComment { .. }
            | Self::RemoteReviewSummaryLine { .. }
            | Self::FileHeader { .. }
            | Self::ReviewedBanner { .. }
            | Self::FileComment { .. }
            | Self::Expander { .. }
            | Self::HiddenLines { .. }
            | Self::ExpandedContext { .. }
            | Self::LineComment { .. }
            | Self::RemoteThreadLine { .. }
            | Self::BinaryOrEmpty { .. }
            | Self::Spacing => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::render_perf_tests::{app_with, app_with_backend, file};
    use crate::model::{FilePatch, FileStatus};
    use crate::syntax::SyntaxHighlighter;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn uncolored_file(path: &str, lines_per_file: usize) -> DiffFile {
        let mut file = file(path, lines_per_file);
        for line in &mut file.hunks[0].lines {
            line.highlighted_spans = None;
        }
        file
    }

    fn is_colored(app: &App, file_idx: usize) -> bool {
        app.diff_files[file_idx].hunks[0]
            .lines
            .iter()
            .any(|l| l.highlighted_spans.is_some())
    }

    #[test]
    fn should_color_the_hunks_on_screen_and_leave_distant_ones_alone() {
        let files = (0..40)
            .map(|i| uncolored_file(&format!("src/f{i}.rs"), 20))
            .collect();
        let mut app = app_with(files);

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .expect("draw");

        assert!(is_colored(&app, 0), "the first file is on screen");
        assert!(!is_colored(&app, 39), "the last file is 800 rows away");
    }

    /// Leave a container-grammar hunk (Vue, PHP, ...) uncolored when the whole-file
    /// pass could not color it. These grammars switch language partway down a file,
    /// so coloring one hunk alone gives it the wrong colors.
    #[test]
    fn should_leave_container_grammar_hunks_uncolored() {
        let mut app = app_with(vec![uncolored_file("src/App.vue", 5)]);

        app.color_visible_hunks(24);

        assert!(!is_colored(&app, 0));
    }

    /// Without a full-file pass (PR mode, --file, files over 1MB) a docstring
    /// hunk still gets hunk-local colors, as it did before the full-file pass existed.
    #[test]
    fn should_color_python_docstring_hunks_when_no_full_file_pass_ran() {
        let mut file = uncolored_file("src/mod.py", 5);
        file.hunks[0].lines[2].content = "    \"\"\"".to_string();
        let mut app = app_with(vec![file]);

        app.color_visible_hunks(24);

        assert!(is_colored(&app, 0));
    }

    const DOCSTRING_SOURCE: [&str; 5] = [
        "class A:",
        "    \"\"\"Doc",
        "    still doc",
        "    \"\"\"",
        "    x = 1",
    ];

    struct FileContentVcs {
        info: VcsInfo,
        lines: Option<Vec<String>>,
        fetches: Arc<AtomicUsize>,
    }

    impl VcsBackend for FileContentVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }
        fn get_working_tree_diff(
            &self,
            _: &SyntaxHighlighter,
        ) -> crate::error::Result<Vec<DiffFile>> {
            Ok(Vec::new())
        }
        fn fetch_context_lines(
            &self,
            _: &Path,
            _: FileStatus,
            _: Option<&str>,
            start: u32,
            end: u32,
        ) -> crate::error::Result<Vec<DiffLine>> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            let lines = self.lines.as_ref().ok_or_else(|| {
                crate::error::TuicrError::UnsupportedOperation("no content".into())
            })?;
            Ok((start..=end)
                .filter_map(|n| {
                    let content = lines.get(n as usize - 1)?.clone();
                    Some(DiffLine {
                        origin: LineOrigin::Context,
                        content,
                        old_lineno: Some(n),
                        new_lineno: Some(n),
                        highlighted_spans: None,
                    })
                })
                .collect())
        }
        fn file_line_count(
            &self,
            _: &Path,
            _: FileStatus,
            _: Option<&str>,
        ) -> crate::error::Result<u32> {
            Ok(0)
        }
    }

    /// A hunk showing only lines 3-5 of `DOCSTRING_SOURCE`: it starts inside the
    /// docstring, so a hunk-local parse reads the closing `"""` as an opener
    /// and colors `x = 1` as string text.
    fn docstring_tail_file() -> DiffFile {
        let mut file = uncolored_file("src/mod.py", 3);
        for (line, n) in file.hunks[0].lines.iter_mut().zip(3u32..) {
            line.origin = LineOrigin::Context;
            line.content = DOCSTRING_SOURCE[n as usize - 1].to_string();
            line.old_lineno = Some(n);
            line.new_lineno = Some(n);
        }
        file
    }

    fn app_with_content(lines: Option<Vec<String>>, file: DiffFile) -> (App, Arc<AtomicUsize>) {
        let fetches = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&fetches);
        let app = app_with_backend(
            move |info| {
                Box::new(FileContentVcs {
                    info,
                    lines,
                    fetches: counter,
                })
            },
            vec![file],
        );
        (app, fetches)
    }

    fn full_file_signatures() -> Vec<String> {
        let source: Vec<String> = DOCSTRING_SOURCE.iter().map(|l| l.to_string()).collect();
        let highlighted = SyntaxHighlighter::default()
            .highlight_file_lines(Path::new("src/mod.py"), &source)
            .expect("python grammar");
        highlighted[2..]
            .iter()
            .map(|spans| {
                spans
                    .as_ref()
                    .expect("spans")
                    .iter()
                    .map(|(style, text)| format!("{text:?}[{:?}/{:?}]", style.fg, style.bg))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn should_color_a_docstring_hunk_using_the_lines_before_it() {
        let source = DOCSTRING_SOURCE.iter().map(|l| l.to_string()).collect();
        let (mut app, _) = app_with_content(Some(source), docstring_tail_file());

        app.color_visible_hunks(24);

        assert_eq!(
            span_signatures(&app.diff_files[0].hunks[0]),
            full_file_signatures()
        );
    }

    #[test]
    fn should_fetch_the_file_once_for_all_of_its_hunks() {
        let source = DOCSTRING_SOURCE.iter().map(|l| l.to_string()).collect();
        let mut file = docstring_tail_file();
        file.hunks.push(file.hunks[0].clone());
        let (mut app, fetches) = app_with_content(Some(source), file);

        app.color_visible_hunks(24);

        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        assert!(
            app.diff_files[0]
                .hunks
                .iter()
                .all(|h| h.lines.iter().all(|l| l.highlighted_spans.is_some()))
        );
    }

    #[test]
    fn should_color_deleted_lines_of_a_docstring_hunk() {
        let source = DOCSTRING_SOURCE.iter().map(|l| l.to_string()).collect();
        let mut file = docstring_tail_file();
        file.hunks[0].lines[2].origin = LineOrigin::Deletion;
        file.hunks[0].lines[2].new_lineno = None;
        let (mut app, _) = app_with_content(Some(source), file);

        app.color_visible_hunks(24);

        assert!(
            app.diff_files[0].hunks[0]
                .lines
                .iter()
                .all(|l| l.highlighted_spans.is_some())
        );
    }

    #[test]
    fn should_fall_back_to_hunk_colors_when_the_file_cannot_be_fetched() {
        let (mut app, fetches) = app_with_content(None, docstring_tail_file());

        app.color_visible_hunks(24);

        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        assert!(is_colored(&app, 0));
        assert_ne!(
            span_signatures(&app.diff_files[0].hunks[0]),
            full_file_signatures(),
            "hunk-local colors differ from full-file colors, so the test above can tell them apart"
        );
    }

    #[test]
    fn should_color_python_hunks_without_a_docstring_delimiter() {
        let mut app = app_with(vec![uncolored_file("src/mod.py", 5)]);

        app.color_visible_hunks(24);

        assert!(is_colored(&app, 0));
    }

    #[test]
    fn should_not_revisit_a_hunk_no_grammar_can_color() {
        let mut app = app_with(vec![uncolored_file("notes.zzzzz", 5)]);

        assert_eq!(app.color_visible_hunks(24), 1, "first pass visits the hunk");
        assert_eq!(app.color_visible_hunks(24), 0, "second pass skips it");
    }

    #[test]
    fn should_revisit_hunks_after_the_annotations_are_rebuilt() {
        let mut app = app_with(vec![uncolored_file("notes.zzzzz", 5)]);
        app.color_visible_hunks(24);

        app.rebuild_annotations();

        assert_eq!(app.color_visible_hunks(24), 1);
    }

    /// One `HunkHeader` per index, so the expected result for any window `[start, end)`
    /// is that range of `(index, 0)` pairs.
    fn one_hunk_per_annotation(count: usize) -> Vec<AnnotatedLine> {
        (0..count)
            .map(|i| AnnotatedLine::HunkHeader {
                file_idx: i,
                hunk_idx: 0,
            })
            .collect()
    }

    /// `visible_hunk_indices` drops only adjacent repeats
    /// Hunk rows form one unbroken run once non-hunk rows are removed.
    /// Check that against real annotations: two hunks per file with a gap and a
    /// comment in the middle of a hunk, in both view modes.
    #[test]
    fn should_keep_each_hunk_in_one_contiguous_run_of_annotations() {
        use crate::model::{Comment, CommentType, LineRange, LineSide};
        use std::collections::HashSet;

        let files: Vec<DiffFile> = (0..3)
            .map(|i| {
                let mut f = file(&format!("src/f{i}.rs"), 10);
                let mut second = f.hunks[0].clone();
                second.old_start = 50;
                second.new_start = 50;
                for (n, line) in second.lines.iter_mut().enumerate() {
                    line.old_lineno = Some(50 + n as u32);
                    line.new_lineno = Some(50 + n as u32);
                }
                f.hunks.push(second);
                f
            })
            .collect();

        for mode in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
            let mut app = app_with(files.clone());
            app.diff_view_mode = mode;
            for f in &files {
                app.session.add_diff_file(f);
                let review = app
                    .session
                    .get_file_mut(f.display_path())
                    .expect("registered file");
                let mut comment = Comment::new(
                    "mid-hunk".into(),
                    CommentType::default(),
                    Some(LineSide::New),
                );
                comment.line_range = Some(LineRange::single(4));
                review.add_line_comment(4, comment);
            }
            app.rebuild_annotations();

            assert!(
                app.line_annotations
                    .iter()
                    .any(|a| matches!(a, AnnotatedLine::LineComment { .. })),
                "{mode:?}: no comment row was rendered, so nothing interrupted a hunk"
            );
            let mut runs: Vec<(usize, usize)> = app
                .line_annotations
                .iter()
                .filter_map(AnnotatedLine::hunk_ref)
                .collect();
            runs.dedup();
            let distinct: HashSet<_> = runs.iter().collect();
            assert_eq!(
                distinct.len(),
                runs.len(),
                "{mode:?}: a hunk's rows are split into more than one run"
            );
            assert_eq!(runs.len(), 6, "{mode:?}: expected three files of two hunks");
        }
    }

    #[test]
    fn should_compute_the_visible_window_for_each_case() {
        let cases: [(&str, usize, usize, usize, std::ops::Range<usize>); 6] = [
            ("offset zero", 10, 0, 3, 0..6),
            (
                "deep offset with a live backward margin",
                100,
                50,
                10,
                40..70,
            ),
            (
                "offset past the end clamps to the bottom",
                20,
                25,
                5,
                15..20,
            ),
            ("fewer annotations than one screen", 3, 0, 10, 0..3),
            ("no annotations", 0, 0, 10, 0..0),
            ("zero rows still covers one row either side", 10, 5, 0, 4..7),
        ];

        for (name, count, offset, rows, expected) in cases {
            let mut app = app_with(Vec::new());
            app.line_annotations = one_hunk_per_annotation(count);
            app.diff_state.scroll_offset = offset;

            let expected: Vec<(usize, usize)> = expected.map(|i| (i, 0)).collect();
            assert_eq!(app.visible_hunk_indices(rows), expected, "case: {name}");
        }
    }

    /// Make sure that comment boxes aren't accidentally colored.
    #[test]
    fn should_color_hunks_revealed_by_comment_input_auto_scroll() {
        let files = (0..80)
            .map(|i| uncolored_file(&format!("src/f{i}.rs"), 20))
            .collect();
        let mut app = app_with(files);
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .expect("draw");
        let far_file = 60;
        assert!(!is_colored(&app, far_file), "far file must start uncolored");

        app.diff_state.current_file_idx = far_file;
        app.enter_comment_mode(true, None);
        app.comment_buffer = "note".to_string();
        app.comment_cursor = app.comment_buffer.len();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .expect("draw");

        assert_ne!(
            app.diff_state.scroll_offset, 0,
            "the comment box must pull the viewport"
        );
        assert!(
            is_colored(&app, far_file),
            "the file the viewport moved to is uncolored"
        );
    }

    fn span_signatures(hunk: &DiffHunk) -> Vec<String> {
        hunk.lines
            .iter()
            .map(|line| match &line.highlighted_spans {
                None => "<uncolored>".to_string(),
                Some(spans) => spans
                    .iter()
                    .map(|(style, text)| format!("{text:?}[{:?}/{:?}]", style.fg, style.bg))
                    .collect::<Vec<_>>()
                    .join(" "),
            })
            .collect()
    }

    /// Used to prove that coloring happens at render time now.
    const PARSE_TIME_SPANS: [&str; 4] = [
        r#""pub"[Some(Rgb(204, 153, 204))/None] " "[Some(Rgb(211, 208, 200))/None] "fn"[Some(Rgb(204, 153, 204))/None] " "[Some(Rgb(211, 208, 200))/None] "alpha"[Some(Rgb(102, 153, 204))/None] "("[Some(Rgb(211, 208, 200))/None] "x"[Some(Rgb(242, 119, 122))/None] ":"[Some(Rgb(211, 208, 200))/None] " "[Some(Rgb(211, 208, 200))/None] "u32"[Some(Rgb(204, 153, 204))/None] ")"[Some(Rgb(211, 208, 200))/None] " "[Some(Rgb(211, 208, 200))/None] "->"[Some(Rgb(211, 208, 200))/None] " "[Some(Rgb(211, 208, 200))/None] "u32"[Some(Rgb(204, 153, 204))/None] " "[Some(Rgb(211, 208, 200))/None] "{"[Some(Rgb(211, 208, 200))/None]"#,
        r#""    x "[Some(Rgb(211, 208, 200))/Some(Rgb(45, 0, 0))] "+"[Some(Rgb(211, 208, 200))/Some(Rgb(45, 0, 0))] " "[Some(Rgb(211, 208, 200))/Some(Rgb(45, 0, 0))] "1"[Some(Rgb(249, 145, 87))/Some(Rgb(45, 0, 0))]"#,
        r#""    x "[Some(Rgb(211, 208, 200))/Some(Rgb(0, 35, 12))] "*"[Some(Rgb(211, 208, 200))/Some(Rgb(0, 35, 12))] " "[Some(Rgb(211, 208, 200))/Some(Rgb(0, 35, 12))] "2"[Some(Rgb(249, 145, 87))/Some(Rgb(0, 35, 12))]"#,
        r#""}"[Some(Rgb(211, 208, 200))/None]"#,
    ];

    fn assert_uncolored(files: &[DiffFile], label: &str) {
        assert!(!files.is_empty(), "{label}: no files parsed");
        for file in files {
            assert!(
                file.hunks
                    .iter()
                    .flat_map(|h| &h.lines)
                    .all(|l| l.highlighted_spans.is_none()),
                "{label}: {} was colored before rendering",
                file.display_path().display()
            );
        }
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn should_leave_hunks_uncolored_after_the_shared_parser() {
        let files = crate::vcs::diff_parser::parse_file_patches(
            vec![FilePatch::new(
                Some(PathBuf::from("sample.rs")),
                Some(PathBuf::from("sample.rs")),
                FileStatus::Modified,
                "@@ -1 +1 @@\n-fn a() {}\n+fn b() {}\n",
            )],
            &SyntaxHighlighter::default(),
        )
        .expect("parse");

        assert_uncolored(&files, "shared parser");
    }

    #[test]
    fn should_leave_hunks_uncolored_after_both_git_backends() {
        let dir = tempfile::tempdir().expect("temp dir");
        run_git(dir.path(), &["init"]);
        run_git(dir.path(), &["config", "user.name", "Tuicr Test"]);
        run_git(dir.path(), &["config", "user.email", "tuicr@example.com"]);
        std::fs::write(dir.path().join("a.rs"), "").expect("write");
        run_git(dir.path(), &["add", "-A"]);
        run_git(dir.path(), &["commit", "-m", "base"]);
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").expect("write");
        // Untracked files take a separate path through the Git CLI backend.
        std::fs::write(dir.path().join("new.rs"), "fn added() {}\n").expect("write");

        for (label, preference) in [
            ("libgit2", crate::vcs::GitBackendPreference::Libgit2),
            ("git cli", crate::vcs::GitBackendPreference::Cli),
        ] {
            let backend = crate::vcs::GitBackend::discover_from(
                dir.path(),
                preference,
                crate::vcs::DiffWhitespaceMode::Normal,
            )
            .expect("open repo");
            let files = backend
                .get_working_tree_diff(&SyntaxHighlighter::default())
                .expect("diff");
            assert_eq!(files.len(), 2, "{label}: expected a.rs and new.rs");
            assert_uncolored(&files, label);
        }
    }

    #[test]
    fn should_leave_hunks_uncolored_after_building_an_all_files_review() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().canonicalize().expect("canonicalize");
        let path = root.join("hello.rs");
        std::fs::write(&path, "fn main() {}\n").expect("write");
        let backend = crate::vcs::FileBackend::new_pristine(vec![path], root).expect("backend");

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("diff");

        assert_uncolored(&files, "all files");
    }

    #[test]
    fn should_color_a_hunk_exactly_as_parsing_did() {
        let patch = "@@ -1,3 +1,3 @@\n pub fn alpha(x: u32) -> u32 {\n-    x + 1\n+    x * 2\n }\n";
        let mut files = crate::vcs::diff_parser::parse_file_patches(
            vec![FilePatch::new(
                Some(PathBuf::from("sample.rs")),
                Some(PathBuf::from("sample.rs")),
                FileStatus::Modified,
                patch,
            )],
            &SyntaxHighlighter::default(),
        )
        .expect("parse");
        let mut hunk = files.remove(0).hunks.remove(0);

        SyntaxHighlighter::default().rehighlight_hunk_in_place(&mut hunk, Path::new("sample.rs"));

        assert_eq!(span_signatures(&hunk), PARSE_TIME_SPANS);
    }
}
