//! Theme support for tuicr
//!
//! Provides dark and light themes with automatic terminal background detection.

use std::sync::OnceLock;

use ratatui::style::Color;

use crate::syntax::SyntaxHighlighter;

/// Complete color theme for the application
pub struct Theme {
    /// Cached syntax highlighter (lazily initialized)
    highlighter: OnceLock<SyntaxHighlighter>,

    // Base colors
    pub bg_highlight: Color,
    pub fg_primary: Color,
    pub fg_secondary: Color,
    pub fg_dim: Color,

    // Diff colors
    pub diff_add: Color,
    pub diff_add_bg: Color,
    pub diff_del: Color,
    pub diff_del_bg: Color,
    pub diff_context: Color,
    pub diff_hunk_header: Color,
    pub expanded_context_fg: Color,

    // Syntax highlighting diff backgrounds (for syntax-highlighted code)
    pub syntax_add_bg: Color,
    pub syntax_del_bg: Color,

    // Syntect theme name for syntax highlighting
    pub syntect_theme: &'static str,

    // File status colors
    pub file_added: Color,
    pub file_modified: Color,
    pub file_deleted: Color,
    pub file_renamed: Color,

    // Review status colors
    pub reviewed: Color,
    pub pending: Color,

    // Comment type colors
    pub comment_note: Color,
    pub comment_suggestion: Color,
    pub comment_issue: Color,
    pub comment_praise: Color,

    // UI element colors
    pub border_focused: Color,
    pub border_unfocused: Color,
    pub status_bar_bg: Color,
    pub cursor_color: Color,

    // Mode indicator colors
    pub mode_fg: Color,
    pub mode_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Create the dark theme (current default colors)
    pub fn dark() -> Self {
        Self {
            highlighter: OnceLock::new(),

            // Base colors
            bg_highlight: Color::Rgb(70, 70, 70),
            fg_primary: Color::White,
            fg_secondary: Color::Rgb(210, 210, 210),
            fg_dim: Color::Rgb(160, 160, 160),

            // Diff colors
            diff_add: Color::Rgb(80, 220, 120),
            diff_add_bg: Color::Rgb(0, 60, 20),
            diff_del: Color::Rgb(240, 90, 90),
            diff_del_bg: Color::Rgb(70, 0, 0),
            diff_context: Color::Rgb(200, 200, 200),
            diff_hunk_header: Color::Rgb(90, 200, 255),
            expanded_context_fg: Color::Rgb(140, 140, 140),

            // Syntax highlighting diff backgrounds
            syntax_add_bg: Color::Rgb(0, 35, 12),
            syntax_del_bg: Color::Rgb(45, 0, 0),

            // Syntect theme for syntax highlighting
            syntect_theme: "base16-eighties.dark",

            // File status colors
            file_added: Color::Rgb(80, 220, 120),
            file_modified: Color::Rgb(255, 210, 90),
            file_deleted: Color::Rgb(240, 90, 90),
            file_renamed: Color::Rgb(255, 140, 220),

            // Review status colors
            reviewed: Color::Rgb(80, 220, 120),
            pending: Color::Rgb(255, 210, 90),

            // Comment type colors
            comment_note: Color::Rgb(90, 170, 255),
            comment_suggestion: Color::Rgb(90, 220, 240),
            comment_issue: Color::Rgb(240, 90, 90),
            comment_praise: Color::Rgb(80, 220, 120),

            // UI element colors
            border_focused: Color::Rgb(90, 200, 255),
            border_unfocused: Color::Rgb(110, 110, 110),
            status_bar_bg: Color::Rgb(30, 30, 30),
            cursor_color: Color::Rgb(255, 210, 90),

            // Mode indicator colors
            mode_fg: Color::Black,
            mode_bg: Color::Rgb(90, 200, 255),
        }
    }

    /// Create the light theme (optimized for light terminal backgrounds)
    pub fn light() -> Self {
        Self {
            highlighter: OnceLock::new(),

            // Base colors - dark text on light background
            bg_highlight: Color::Rgb(200, 200, 220),
            fg_primary: Color::Rgb(0, 0, 0),
            fg_secondary: Color::Rgb(30, 30, 30),
            fg_dim: Color::Rgb(80, 80, 80),

            // Diff colors - subtle backgrounds, dark text
            // Key: backgrounds should be very light, text should be dark
            diff_add: Color::Rgb(0, 80, 0),         // Dark green text
            diff_add_bg: Color::Rgb(220, 255, 220), // Very light green bg
            diff_del: Color::Rgb(120, 0, 0),        // Dark red text
            diff_del_bg: Color::Rgb(255, 240, 240), // Very light pink bg
            diff_context: Color::Rgb(0, 0, 0),      // Black for max readability
            diff_hunk_header: Color::Rgb(0, 60, 140),
            expanded_context_fg: Color::Rgb(60, 60, 60),

            // Syntax highlighting diff backgrounds (lighter for light theme)
            syntax_add_bg: Color::Rgb(220, 255, 220), // Very light green
            syntax_del_bg: Color::Rgb(255, 230, 230), // Very light pink

            // Syntect theme for syntax highlighting (light variant)
            syntect_theme: "base16-ocean.light",

            // File status colors
            file_added: Color::Rgb(0, 100, 0),
            file_modified: Color::Rgb(140, 80, 0),
            file_deleted: Color::Rgb(160, 0, 0),
            file_renamed: Color::Rgb(100, 0, 100),

            // Review status colors
            reviewed: Color::Rgb(0, 100, 0),
            pending: Color::Rgb(140, 80, 0),

            // Comment type colors
            comment_note: Color::Rgb(0, 60, 140),
            comment_suggestion: Color::Rgb(0, 100, 120),
            comment_issue: Color::Rgb(160, 0, 0),
            comment_praise: Color::Rgb(0, 100, 0),

            // UI element colors
            border_focused: Color::Rgb(0, 60, 140),
            border_unfocused: Color::Rgb(100, 100, 100),
            status_bar_bg: Color::Rgb(210, 210, 220),
            cursor_color: Color::Rgb(140, 80, 0),

            // Mode indicator colors
            mode_fg: Color::White,
            mode_bg: Color::Rgb(0, 80, 160),
        }
    }
}

/// Theme selection from CLI argument
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeArg {
    #[default]
    Dark,
    Light,
}

impl ThemeArg {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

/// Resolve a theme based on the CLI argument
pub fn resolve_theme(arg: ThemeArg) -> Theme {
    match arg {
        ThemeArg::Dark => Theme::dark(),
        ThemeArg::Light => Theme::light(),
    }
}

impl Theme {
    /// Get the syntax highlighter for this theme (lazily initialized, cached)
    pub fn syntax_highlighter(&self) -> &SyntaxHighlighter {
        self.highlighter.get_or_init(|| {
            SyntaxHighlighter::new(self.syntect_theme, self.syntax_add_bg, self.syntax_del_bg)
        })
    }
}

/// Print help message and exit
fn print_help() -> ! {
    let name = std::env::args()
        .next()
        .and_then(|p| {
            std::path::Path::new(&p)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "tuicr".to_string());
    println!(
        "tuicr - Review AI-generated diffs like a GitHub pull request

Usage: {} [OPTIONS]

Options:
  --theme <THEME>  Color theme to use [default: dark]
                   Valid values: dark, light
  -h, --help       Print this help message

Press ? in the application for keybinding help.",
        name
    );
    std::process::exit(0);
}

/// Parse --theme argument from command line
///
/// We use a handrolled argument parser instead of clap to keep binary size
/// small and build times fast. If we end up needing more complex argument
/// handling, we can revisit this decision.
pub fn parse_theme_arg() -> ThemeArg {
    let args: Vec<String> = std::env::args().collect();

    for i in 0..args.len() {
        // Handle --help / -h
        if args[i] == "--help" || args[i] == "-h" {
            print_help();
        }

        // Handle --theme value
        if args[i] == "--theme" {
            if let Some(value) = args.get(i + 1) {
                return ThemeArg::from_str(value).unwrap_or_else(|| {
                    eprintln!(
                        "Warning: Unknown theme '{}', using dark. Valid options: dark, light",
                        value
                    );
                    ThemeArg::Dark
                });
            } else {
                eprintln!("Warning: --theme requires a value (dark, light)");
                return ThemeArg::Dark;
            }
        }
        // Handle --theme=value
        if let Some(value) = args[i].strip_prefix("--theme=") {
            return ThemeArg::from_str(value).unwrap_or_else(|| {
                eprintln!(
                    "Warning: Unknown theme '{}', using dark. Valid options: dark, light",
                    value
                );
                ThemeArg::Dark
            });
        }
    }

    ThemeArg::Dark
}
