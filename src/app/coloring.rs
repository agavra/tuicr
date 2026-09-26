use super::*;
use crate::syntax::SyntaxHighlighter;

impl App {
    /// Color uncolored visible hunks. Call this before building a line.
    ///
    /// Returns how many hunks it visited.
    ///
    /// `max_rows` is the frame's height.
    /// `diff_state.viewport_height` is written during render.
    pub(crate) fn color_visible_hunks(&mut self, max_rows: usize) -> usize {
        let highlighter = self.theme.syntax_highlighter();
        let unvisited: Vec<(usize, usize)> = self
            .visible_hunk_indices(max_rows)
            .into_iter()
            .filter(|key| self.colored_hunks.insert(*key))
            .collect();

        unvisited
            .into_iter()
            .filter_map(|(file_idx, hunk_idx)| {
                let DiffFile {
                    old_path,
                    new_path,
                    hunks,
                    ..
                } = self.diff_files.get_mut(file_idx)?;
                let path = new_path.as_deref().or(old_path.as_deref())?;
                let hunk = hunks.get_mut(hunk_idx)?;

                // only color uncolored lines
                if hunk.lines.iter().any(|l| l.highlighted_spans.is_some()) {
                    return Some(());
                }
                color_hunk(hunk, path, highlighter);
                Some(())
            })
            .count()
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

// Color a single hunk
fn color_hunk(hunk: &mut DiffHunk, syntax_path: &Path, highlighter: &SyntaxHighlighter) {
    let contents: Vec<String> = hunk.lines.iter().map(|l| l.content.clone()).collect();
    let origins: Vec<LineOrigin> = hunk.lines.iter().map(|l| l.origin).collect();
    let seq = SyntaxHighlighter::split_diff_lines_for_highlighting(&contents, &origins);
    let old = highlighter.highlight_file_lines(syntax_path, &seq.old_lines);
    let new = highlighter.highlight_file_lines(syntax_path, &seq.new_lines);

    let indices = seq.old_line_indices.iter().zip(&seq.new_line_indices);
    for (line, (&old_idx, &new_idx)) in hunk.lines.iter_mut().zip(indices) {
        line.highlighted_spans = highlighter.highlighted_line_for_diff_with_background(
            old.as_deref(),
            new.as_deref(),
            old_idx,
            new_idx,
            line.origin,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::render_perf_tests::{app_with, file};
    use crate::model::{FilePatch, FileStatus};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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

        color_hunk(
            &mut hunk,
            Path::new("sample.rs"),
            &SyntaxHighlighter::default(),
        );

        assert_eq!(span_signatures(&hunk), PARSE_TIME_SPANS);
    }
}
