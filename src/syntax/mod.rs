use ratatui::style::{Color, Modifier, Style};
use std::path::Path;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use crate::model::diff_types::LineOrigin;

/// Helper to highlight lines of code from a diff
pub struct SyntaxHighlighter {
    pub syntax_set: SyntaxSet,
    pub theme: syntect::highlighting::Theme,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();

        // Prefer a higher-contrast built-in theme, fall back to the previous default.
        let theme = theme_set
            .themes
            .get("base16-eighties.dark")
            .or_else(|| theme_set.themes.get("base16-ocean.dark"))
            .cloned()
            .unwrap_or_default();

        Self { syntax_set, theme }
    }

    /// Highlight all lines in a file's content
    /// Returns a vector of styled spans for each line
    pub fn highlight_file_lines(
        &self,
        file_path: &Path,
        lines: &[String],
    ) -> Option<Vec<Vec<(Style, String)>>> {
        use syntect::easy::HighlightLines;

        // Get syntax definition
        let syntax = self.get_syntax(file_path)?;

        // Create highlighter
        let mut highlighter = HighlightLines::new(syntax, &self.theme);

        let mut result = Vec::new();

        for line in lines {
            // Highlight the line
            let ranges = highlighter.highlight_line(line, &self.syntax_set).ok()?;

            // Convert syntect styles to ratatui styles
            let spans: Vec<(Style, String)> = ranges
                .into_iter()
                .map(|(style, text)| {
                    let fg_color =
                        Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);

                    let mut ratatui_style = Style::default().fg(fg_color);

                    // Apply font style modifiers
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

                    (ratatui_style, text.to_string())
                })
                .collect();

            result.push(spans);
        }

        Some(result)
    }

    /// Get syntax definition from file path
    fn get_syntax(&self, file_path: &Path) -> Option<&syntect::parsing::SyntaxReference> {
        // Try by extension first
        if let Some(ext) = file_path.extension().and_then(|e| e.to_str())
            && let Some(syntax) = self.syntax_set.find_syntax_by_extension(ext)
        {
            return Some(syntax);
        }

        // Try by filename (for files like Makefile, Dockerfile, etc.)
        if let Some(filename) = file_path.file_name().and_then(|f| f.to_str())
            && let Some(syntax) = self.syntax_set.find_syntax_by_name(filename)
        {
            return Some(syntax);
        }

        None
    }

    /// Apply diff background colors to highlighted spans based on line origin
    pub fn apply_diff_background(
        spans: Vec<(Style, String)>,
        origin: LineOrigin,
    ) -> Vec<(Style, String)> {
        let bg_color = match origin {
            LineOrigin::Addition => Color::Rgb(0, 35, 12),
            LineOrigin::Deletion => Color::Rgb(45, 0, 0),
            LineOrigin::Context => return spans, // No background for context
        };

        spans
            .into_iter()
            .map(|(style, text)| (style.bg(bg_color), text))
            .collect()
    }
}

impl Default for SyntaxHighlighter {
    fn default() -> Self {
        Self::new()
    }
}
