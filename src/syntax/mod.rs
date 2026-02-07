use ratatui::style::{Color, Modifier, Style};
use std::path::Path;
use two_face::theme::EmbeddedThemeName;

use crate::model::diff_types::LineOrigin;

/// A single line of highlighted spans (style + text pairs).
type HighlightedSpans = Vec<(Style, String)>;

/// Per-line highlight results for a file: `Some` if the line was highlighted, `None` on failure.
type HighlightedLines = Vec<Option<HighlightedSpans>>;

/// Helper to highlight lines of code from a diff
pub struct SyntaxHighlighter {
    pub syntax_set: syntect::parsing::SyntaxSet,
    pub theme: syntect::highlighting::Theme,
    /// Background color for added lines
    pub add_bg: Color,
    /// Background color for deleted lines
    pub del_bg: Color,
}

pub(crate) struct DiffHighlightSequences {
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    pub old_line_indices: Vec<Option<usize>>,
    pub new_line_indices: Vec<Option<usize>>,
}

impl Default for SyntaxHighlighter {
    fn default() -> Self {
        Self::new(
            EmbeddedThemeName::Base16EightiesDark,
            Color::Rgb(0, 35, 12),
            Color::Rgb(45, 0, 0),
        )
    }
}

impl SyntaxHighlighter {
    /// Create a new syntax highlighter with the given theme and diff background colors
    pub fn new(theme_name: EmbeddedThemeName, add_bg: Color, del_bg: Color) -> Self {
        let syntax_set = two_face::syntax::extra_newlines();
        let theme_set = two_face::theme::extra();
        let theme = theme_set[theme_name].clone();

        Self {
            syntax_set,
            theme,
            add_bg,
            del_bg,
        }
    }

    /// Highlight all lines in a file's content.
    ///
    /// Returns `None` when no syntax can be resolved for the file (by path or shebang).
    /// Otherwise returns one entry per input line:
    /// - `Some(spans)` if that line was highlighted successfully (including empty spans)
    /// - `None` if highlighting failed for that specific line
    pub fn highlight_file_lines(
        &self,
        file_path: &Path,
        lines: &[String],
    ) -> Option<HighlightedLines> {
        use syntect::easy::HighlightLines;

        // Get syntax definition
        let syntax = self.get_syntax(file_path).or_else(|| {
            lines
                .first()
                .and_then(|line| self.syntax_set.find_syntax_by_first_line(line))
        })?;

        // Create highlighter
        let mut highlighter = HighlightLines::new(syntax, &self.theme);

        Some(Self::collect_line_highlights(lines, |line| {
            // Highlight failures are scoped to the single line; other lines still keep highlighting.
            highlighter
                .highlight_line(line, &self.syntax_set)
                .ok()
                .map(|ranges| {
                    ranges
                        .into_iter()
                        .map(|(style, text)| {
                            (Self::syntect_to_ratatui_style(style), text.to_string())
                        })
                        .collect()
                })
        }))
    }

    fn collect_line_highlights<F>(lines: &[String], mut highlight_line: F) -> HighlightedLines
    where
        F: FnMut(&str) -> Option<HighlightedSpans>,
    {
        let mut result = Vec::with_capacity(lines.len());
        for line in lines {
            result.push(highlight_line(line));
        }
        result
    }

    fn highlighted_line_at(
        highlighted_lines: Option<&[Option<HighlightedSpans>]>,
        line_idx: Option<usize>,
    ) -> Option<HighlightedSpans> {
        line_idx
            .and_then(|idx| highlighted_lines.and_then(|all| all.get(idx)))
            .and_then(|line_highlight| line_highlight.as_ref().cloned())
    }

    pub(crate) fn split_diff_lines_for_highlighting(
        line_contents: &[String],
        line_origins: &[LineOrigin],
    ) -> DiffHighlightSequences {
        debug_assert_eq!(line_contents.len(), line_origins.len());

        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        let mut old_line_indices = Vec::with_capacity(line_origins.len());
        let mut new_line_indices = Vec::with_capacity(line_origins.len());

        for (content, origin) in line_contents.iter().zip(line_origins.iter()) {
            match origin {
                LineOrigin::Context => {
                    let old_idx = old_lines.len();
                    old_lines.push(content.clone());
                    old_line_indices.push(Some(old_idx));

                    let new_idx = new_lines.len();
                    new_lines.push(content.clone());
                    new_line_indices.push(Some(new_idx));
                }
                LineOrigin::Addition => {
                    let new_idx = new_lines.len();
                    new_lines.push(content.clone());
                    old_line_indices.push(None);
                    new_line_indices.push(Some(new_idx));
                }
                LineOrigin::Deletion => {
                    let old_idx = old_lines.len();
                    old_lines.push(content.clone());
                    old_line_indices.push(Some(old_idx));
                    new_line_indices.push(None);
                }
            }
        }

        DiffHighlightSequences {
            old_lines,
            new_lines,
            old_line_indices,
            new_line_indices,
        }
    }

    pub(crate) fn highlighted_line_for_diff_with_background(
        &self,
        old_highlighted_lines: Option<&[Option<HighlightedSpans>]>,
        new_highlighted_lines: Option<&[Option<HighlightedSpans>]>,
        old_line_idx: Option<usize>,
        new_line_idx: Option<usize>,
        origin: LineOrigin,
    ) -> Option<HighlightedSpans> {
        let spans = match origin {
            LineOrigin::Addition => Self::highlighted_line_at(new_highlighted_lines, new_line_idx),
            LineOrigin::Deletion => Self::highlighted_line_at(old_highlighted_lines, old_line_idx),
            LineOrigin::Context => Self::highlighted_line_at(new_highlighted_lines, new_line_idx),
        }?;

        Some(self.apply_diff_background(spans, origin))
    }

    fn syntect_to_ratatui_style(style: syntect::highlighting::Style) -> Style {
        let fg_color = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
        let mut ratatui_style = Style::default().fg(fg_color);

        if style
            .font_style
            .contains(syntect::highlighting::FontStyle::BOLD)
        {
            ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
        }
        if style
            .font_style
            .contains(syntect::highlighting::FontStyle::ITALIC)
        {
            ratatui_style = ratatui_style.add_modifier(Modifier::ITALIC);
        }
        if style
            .font_style
            .contains(syntect::highlighting::FontStyle::UNDERLINE)
        {
            ratatui_style = ratatui_style.add_modifier(Modifier::UNDERLINED);
        }

        ratatui_style
    }

    /// Map extensions not in two-face's syntax set to a known equivalent.
    fn fallback_extension(ext: &str) -> Option<&'static str> {
        match ext {
            "jsx" | "mjs" | "cjs" => Some("js"),
            "hbs" | "handlebars" | "mustache" | "ejs" | "pug" | "jade" | "njk" => Some("html"),
            "mdx" => Some("md"),
            "jsonc" | "json5" | "prisma" => Some("json"),
            "heex" => Some("rb"),
            _ => None,
        }
    }

    /// Map extension-less filenames to a known syntax extension.
    fn fallback_filename(name: &str) -> Option<&'static str> {
        match name {
            "Containerfile" => Some("sh"),
            "Justfile" | "justfile" => Some("sh"),
            _ => None,
        }
    }

    /// Resolve syntax from a file path using this lookup order:
    /// extension -> lowercase extension (when different) -> fallback extension ->
    /// filename token -> filename name -> fallback filename.
    fn get_syntax(&self, file_path: &Path) -> Option<&syntect::parsing::SyntaxReference> {
        // Try by extension first
        if let Some(ext) = file_path.extension().and_then(|e| e.to_str()) {
            if let Some(syntax) = self.syntax_set.find_syntax_by_extension(ext) {
                return Some(syntax);
            }

            let normalized = ext.to_ascii_lowercase();
            if normalized != ext
                && let Some(syntax) = self.syntax_set.find_syntax_by_extension(&normalized)
            {
                return Some(syntax);
            }

            // Try fallback mapping for extensions not in syntect's defaults
            if let Some(fallback) = Self::fallback_extension(&normalized)
                && let Some(syntax) = self.syntax_set.find_syntax_by_extension(fallback)
            {
                return Some(syntax);
            }
        }

        // Try token/name matches for extension-less files (e.g. Makefile, BUILD).
        if let Some(filename) = file_path.file_name().and_then(|f| f.to_str()) {
            if let Some(syntax) = self.syntax_set.find_syntax_by_token(filename) {
                return Some(syntax);
            }

            if let Some(syntax) = self.syntax_set.find_syntax_by_name(filename) {
                return Some(syntax);
            }

            if let Some(fallback) = Self::fallback_filename(filename)
                && let Some(syntax) = self.syntax_set.find_syntax_by_extension(fallback)
            {
                return Some(syntax);
            }
        }

        None
    }

    /// Apply diff background colors to highlighted spans based on line origin
    pub fn apply_diff_background(
        &self,
        spans: Vec<(Style, String)>,
        origin: LineOrigin,
    ) -> Vec<(Style, String)> {
        let bg_color = match origin {
            LineOrigin::Addition => self.add_bg,
            LineOrigin::Deletion => self.del_bg,
            LineOrigin::Context => return spans, // No background for context
        };

        spans
            .into_iter()
            .map(|(style, text)| (style.bg(bg_color), text))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_find_syntax_for_uppercase_extension() {
        let highlighter = SyntaxHighlighter::default();
        let syntax = highlighter.get_syntax(Path::new("SRC/MAIN.RS"));
        assert!(syntax.is_some());
    }

    #[test]
    fn should_find_syntax_for_build_filename_token() {
        let highlighter = SyntaxHighlighter::default();
        let syntax = highlighter.get_syntax(Path::new("BUILD"));
        assert!(syntax.is_some());
    }

    #[test]
    fn should_highlight_each_line_independently() {
        let highlighter = SyntaxHighlighter::default();
        let lines = vec![
            "fn main() {".to_string(),
            "    let x = 42;".to_string(),
            "}".to_string(),
        ];
        let highlighted = highlighter.highlight_file_lines(Path::new("main.rs"), &lines);

        assert!(highlighted.is_some());
        let highlighted = highlighted.unwrap();
        assert_eq!(highlighted.len(), lines.len());
        assert!(highlighted.iter().all(|line| line.is_some()));
    }

    #[test]
    fn should_keep_file_highlighting_when_one_line_fails() {
        let lines = vec!["first".to_string(), "bad".to_string(), "third".to_string()];
        let highlighted = SyntaxHighlighter::collect_line_highlights(&lines, |line| {
            if line == "bad" {
                None
            } else {
                Some(vec![(Style::default(), line.to_string())])
            }
        });

        assert_eq!(highlighted.len(), lines.len());
        assert!(highlighted[0].is_some());
        assert!(highlighted[1].is_none());
        assert!(highlighted[2].is_some());
    }

    #[test]
    fn should_find_syntax_for_typescript() {
        let highlighter = SyntaxHighlighter::default();
        for ext in &["ts", "tsx", "mts", "cts", "jsx", "mjs", "cjs"] {
            let path = format!("file.{ext}");
            assert!(
                highlighter.get_syntax(Path::new(&path)).is_some(),
                "should find syntax for .{ext}"
            );
        }
    }

    #[test]
    fn should_find_syntax_for_fallback_extensions() {
        let highlighter = SyntaxHighlighter::default();
        let extensions = [
            "jsx", "mjs", "cjs", "hbs", "mustache", "ejs", "pug", "njk", "mdx", "jsonc", "json5",
            "prisma", "heex",
        ];
        for ext in &extensions {
            let path = format!("file.{ext}");
            assert!(
                highlighter.get_syntax(Path::new(&path)).is_some(),
                "should find syntax for .{ext}"
            );
        }
    }

    #[test]
    fn should_find_syntax_for_fallback_filenames() {
        let highlighter = SyntaxHighlighter::default();
        for name in &["Containerfile", "Justfile", "justfile"] {
            assert!(
                highlighter.get_syntax(Path::new(name)).is_some(),
                "should find syntax for {name}"
            );
        }
    }

    #[test]
    fn highlighted_spans_should_have_color() {
        let highlighter = SyntaxHighlighter::default();
        let lines = vec![
            "fn main() {".to_string(),
            "    let x = 42;".to_string(),
            "}".to_string(),
        ];
        let highlighted = highlighter
            .highlight_file_lines(Path::new("test.rs"), &lines)
            .unwrap();
        for (i, line) in highlighted.iter().enumerate() {
            let spans = line
                .as_ref()
                .unwrap_or_else(|| panic!("line {i} should be Some"));
            assert!(!spans.is_empty(), "line {i} should have spans");
            // At least one span should have a non-default foreground color
            let has_fg = spans.iter().any(|(style, _)| style.fg.is_some());
            assert!(has_fg, "line {i} should have foreground color: {spans:?}");
        }
    }

    #[test]
    fn should_detect_syntax_from_shebang_when_extensionless() {
        let highlighter = SyntaxHighlighter::default();
        let lines = vec![
            "#!/usr/bin/env python".to_string(),
            "print('hello')".to_string(),
        ];

        let highlighted = highlighter.highlight_file_lines(Path::new("script"), &lines);
        assert!(highlighted.is_some());
        assert_eq!(highlighted.unwrap().len(), lines.len());
    }

    #[test]
    fn should_preserve_empty_line_highlight_results() {
        let lines = vec!["value".to_string(), "".to_string()];
        let highlighted = SyntaxHighlighter::collect_line_highlights(&lines, |line| {
            if line.is_empty() {
                Some(Vec::new())
            } else {
                Some(vec![(Style::default(), line.to_string())])
            }
        });

        assert!(matches!(highlighted[1], Some(ref spans) if spans.is_empty()));
    }

    #[test]
    fn should_not_use_weak_fallback_mappings() {
        for ext in &["toml", "hcl", "tf", "tfvars", "nix", "swift", "zig", "v"] {
            assert_eq!(SyntaxHighlighter::fallback_extension(ext), None);
        }
    }

    #[test]
    fn split_diff_lines_for_highlighting_should_build_old_and_new_sequences() {
        let contents = vec![
            "ctx".to_string(),
            "del".to_string(),
            "add".to_string(),
            "ctx2".to_string(),
        ];
        let origins = vec![
            LineOrigin::Context,
            LineOrigin::Deletion,
            LineOrigin::Addition,
            LineOrigin::Context,
        ];

        let seq = SyntaxHighlighter::split_diff_lines_for_highlighting(&contents, &origins);
        assert_eq!(seq.old_lines, vec!["ctx", "del", "ctx2"]);
        assert_eq!(seq.new_lines, vec!["ctx", "add", "ctx2"]);
        assert_eq!(seq.old_line_indices, vec![Some(0), Some(1), None, Some(2)]);
        assert_eq!(seq.new_line_indices, vec![Some(0), None, Some(1), Some(2)]);
    }

    #[test]
    fn highlighted_line_for_diff_with_background_should_handle_none_per_line() {
        let highlighter = SyntaxHighlighter::default();
        let old_lines = vec![None];
        let new_lines = vec![None];
        let highlighted = highlighter.highlighted_line_for_diff_with_background(
            Some(&old_lines),
            Some(&new_lines),
            Some(0),
            Some(0),
            LineOrigin::Addition,
        );
        assert!(highlighted.is_none());
    }

    #[test]
    fn highlighted_line_for_diff_with_background_should_apply_background_on_success() {
        let highlighter = SyntaxHighlighter::default();
        let old_lines = vec![Some(vec![(Style::default(), "old".to_string())])];
        let new_lines = vec![Some(vec![(Style::default(), "new".to_string())])];

        let deletion = highlighter.highlighted_line_for_diff_with_background(
            Some(&old_lines),
            Some(&new_lines),
            Some(0),
            Some(0),
            LineOrigin::Deletion,
        );
        let addition = highlighter.highlighted_line_for_diff_with_background(
            Some(&old_lines),
            Some(&new_lines),
            Some(0),
            Some(0),
            LineOrigin::Addition,
        );
        let context = highlighter.highlighted_line_for_diff_with_background(
            Some(&old_lines),
            Some(&new_lines),
            Some(0),
            Some(0),
            LineOrigin::Context,
        );

        let deletion = deletion.unwrap();
        assert_eq!(deletion.len(), 1);
        assert_eq!(deletion[0].0.bg, Some(highlighter.del_bg));
        assert_eq!(deletion[0].1, "old");

        let addition = addition.unwrap();
        assert_eq!(addition.len(), 1);
        assert_eq!(addition[0].0.bg, Some(highlighter.add_bg));
        assert_eq!(addition[0].1, "new");

        let context = context.unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].0.bg, None);
        assert_eq!(context[0].1, "new");
    }
}
