//! Theme support for tuicr
//!
//! Provides dark and light themes with automatic terminal background detection.

use std::sync::OnceLock;

use ratatui::style::Color;
use two_face::theme::EmbeddedThemeName;

use crate::config::config_path_hint;
use crate::syntax::SyntaxHighlighter;

/// Complete color theme for the application
pub struct Theme {
    /// Cached syntax highlighter (lazily initialized)
    highlighter: OnceLock<SyntaxHighlighter>,

    // Base colors
    pub panel_bg: Color,
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
    pub syntect_theme: EmbeddedThemeName,

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
    pub branch_name: Color,
    pub help_indicator: Color,

    // Message/update badge colors
    pub message_info_fg: Color,
    pub message_info_bg: Color,
    pub message_warning_fg: Color,
    pub message_warning_bg: Color,
    pub message_error_fg: Color,
    pub message_error_bg: Color,
    pub update_badge_fg: Color,
    pub update_badge_bg: Color,

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
            panel_bg: Color::Rgb(24, 24, 28),
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
            syntect_theme: EmbeddedThemeName::Base16EightiesDark,

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
            branch_name: Color::Rgb(90, 220, 240),
            help_indicator: Color::Rgb(110, 110, 110),

            // Message/update badge colors
            message_info_fg: Color::Black,
            message_info_bg: Color::Cyan,
            message_warning_fg: Color::Black,
            message_warning_bg: Color::Rgb(255, 210, 90),
            message_error_fg: Color::White,
            message_error_bg: Color::Rgb(240, 90, 90),
            update_badge_fg: Color::Black,
            update_badge_bg: Color::Rgb(255, 210, 90),

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
            panel_bg: Color::Rgb(245, 243, 232),
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
            syntect_theme: EmbeddedThemeName::Base16OceanLight,

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
            branch_name: Color::Rgb(0, 100, 120),
            help_indicator: Color::Rgb(90, 90, 90),

            // Message/update badge colors
            message_info_fg: Color::Black,
            message_info_bg: Color::Rgb(140, 220, 255),
            message_warning_fg: Color::Black,
            message_warning_bg: Color::Rgb(240, 210, 150),
            message_error_fg: Color::White,
            message_error_bg: Color::Rgb(180, 60, 60),
            update_badge_fg: Color::Black,
            update_badge_bg: Color::Rgb(240, 210, 150),

            // Mode indicator colors
            mode_fg: Color::White,
            mode_bg: Color::Rgb(0, 80, 160),
        }
    }

    pub fn catppuccin_latte() -> Self {
        let flavor = CatppuccinFlavor {
            dark: false,
            text: rgb(76, 79, 105),
            subtext1: rgb(92, 95, 119),
            overlay1: rgb(140, 143, 161),
            overlay0: rgb(156, 160, 176),
            surface2: rgb(172, 176, 190),
            surface1: rgb(188, 192, 204),
            base: rgb(239, 241, 245),
            mantle: rgb(230, 233, 239),
            crust: rgb(220, 224, 232),
            red: rgb(210, 15, 57),
            yellow: rgb(223, 142, 29),
            green: rgb(64, 160, 43),
            teal: rgb(23, 146, 153),
            blue: rgb(30, 102, 245),
            lavender: rgb(114, 135, 253),
            peach: rgb(254, 100, 11),
            pink: rgb(234, 118, 203),
        };
        catppuccin_theme(flavor, EmbeddedThemeName::CatppuccinLatte)
    }

    pub fn catppuccin_frappe() -> Self {
        let flavor = CatppuccinFlavor {
            dark: true,
            text: rgb(198, 208, 245),
            subtext1: rgb(181, 191, 226),
            overlay1: rgb(131, 139, 167),
            overlay0: rgb(115, 121, 148),
            surface2: rgb(98, 104, 128),
            surface1: rgb(81, 87, 109),
            base: rgb(48, 52, 70),
            mantle: rgb(41, 44, 60),
            crust: rgb(35, 38, 52),
            red: rgb(231, 130, 132),
            yellow: rgb(229, 200, 144),
            green: rgb(166, 209, 137),
            teal: rgb(129, 200, 190),
            blue: rgb(140, 170, 238),
            lavender: rgb(186, 187, 241),
            peach: rgb(239, 159, 118),
            pink: rgb(244, 184, 228),
        };
        catppuccin_theme(flavor, EmbeddedThemeName::CatppuccinFrappe)
    }

    pub fn catppuccin_macchiato() -> Self {
        let flavor = CatppuccinFlavor {
            dark: true,
            text: rgb(202, 211, 245),
            subtext1: rgb(184, 192, 224),
            overlay1: rgb(128, 135, 162),
            overlay0: rgb(110, 115, 141),
            surface2: rgb(91, 96, 120),
            surface1: rgb(73, 77, 100),
            base: rgb(36, 39, 58),
            mantle: rgb(30, 32, 48),
            crust: rgb(24, 25, 38),
            red: rgb(237, 135, 150),
            yellow: rgb(238, 212, 159),
            green: rgb(166, 218, 149),
            teal: rgb(139, 213, 202),
            blue: rgb(138, 173, 244),
            lavender: rgb(183, 189, 248),
            peach: rgb(245, 169, 127),
            pink: rgb(245, 189, 230),
        };
        catppuccin_theme(flavor, EmbeddedThemeName::CatppuccinMacchiato)
    }

    pub fn catppuccin_mocha() -> Self {
        let flavor = CatppuccinFlavor {
            dark: true,
            text: rgb(205, 214, 244),
            subtext1: rgb(186, 194, 222),
            overlay1: rgb(127, 132, 156),
            overlay0: rgb(108, 112, 134),
            surface2: rgb(88, 91, 112),
            surface1: rgb(69, 71, 90),
            base: rgb(30, 30, 46),
            mantle: rgb(24, 24, 37),
            crust: rgb(17, 17, 27),
            red: rgb(243, 139, 168),
            yellow: rgb(249, 226, 175),
            green: rgb(166, 227, 161),
            teal: rgb(148, 226, 213),
            blue: rgb(137, 180, 250),
            lavender: rgb(180, 190, 254),
            peach: rgb(250, 179, 135),
            pink: rgb(245, 194, 231),
        };
        catppuccin_theme(flavor, EmbeddedThemeName::CatppuccinMocha)
    }
}

#[derive(Clone, Copy)]
struct CatppuccinFlavor {
    dark: bool,
    text: Color,
    subtext1: Color,
    overlay1: Color,
    overlay0: Color,
    surface2: Color,
    surface1: Color,
    base: Color,
    mantle: Color,
    crust: Color,
    red: Color,
    yellow: Color,
    green: Color,
    teal: Color,
    blue: Color,
    lavender: Color,
    peach: Color,
    pink: Color,
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

fn blend(base: Color, accent: Color, accent_percent: u8) -> Color {
    debug_assert!(accent_percent <= 100);
    match (base, accent) {
        (Color::Rgb(br, bg, bb), Color::Rgb(ar, ag, ab)) => {
            let p = u16::from(accent_percent);
            let inv = 100_u16.saturating_sub(p);
            let mix =
                |b: u8, a: u8| -> u8 { ((u16::from(b) * inv + u16::from(a) * p) / 100) as u8 };
            rgb(mix(br, ar), mix(bg, ag), mix(bb, ab))
        }
        _ => accent,
    }
}

fn catppuccin_theme(flavor: CatppuccinFlavor, syntect_theme: EmbeddedThemeName) -> Theme {
    let accent_fg = if flavor.dark {
        flavor.base
    } else {
        flavor.crust
    };
    let diff_add_bg = blend(flavor.base, flavor.green, 20);
    let diff_del_bg = blend(flavor.base, flavor.red, 20);
    let syntax_add_bg = blend(flavor.base, flavor.green, 16);
    let syntax_del_bg = blend(flavor.base, flavor.red, 16);

    Theme {
        highlighter: OnceLock::new(),

        // Base colors
        panel_bg: flavor.base,
        bg_highlight: flavor.surface1,
        fg_primary: flavor.text,
        fg_secondary: flavor.subtext1,
        fg_dim: flavor.overlay0,

        // Diff colors
        diff_add: flavor.green,
        diff_add_bg,
        diff_del: flavor.red,
        diff_del_bg,
        diff_context: flavor.text,
        diff_hunk_header: flavor.blue,
        expanded_context_fg: flavor.overlay1,

        // Syntax highlighting diff backgrounds
        syntax_add_bg,
        syntax_del_bg,

        // Syntect theme for syntax highlighting
        syntect_theme,

        // File status colors
        file_added: flavor.green,
        file_modified: flavor.yellow,
        file_deleted: flavor.red,
        file_renamed: flavor.pink,

        // Review status colors
        reviewed: flavor.green,
        pending: flavor.yellow,

        // Comment type colors
        comment_note: flavor.blue,
        comment_suggestion: flavor.teal,
        comment_issue: flavor.red,
        comment_praise: flavor.green,

        // UI element colors
        border_focused: flavor.blue,
        border_unfocused: flavor.surface2,
        status_bar_bg: flavor.mantle,
        cursor_color: flavor.peach,
        branch_name: flavor.teal,
        help_indicator: flavor.overlay0,

        // Message/update badge colors
        message_info_fg: accent_fg,
        message_info_bg: flavor.teal,
        message_warning_fg: accent_fg,
        message_warning_bg: flavor.yellow,
        message_error_fg: accent_fg,
        message_error_bg: flavor.red,
        update_badge_fg: accent_fg,
        update_badge_bg: flavor.peach,

        // Mode indicator colors
        mode_fg: accent_fg,
        mode_bg: flavor.lavender,
    }
}

/// Theme selection from CLI argument
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeArg {
    #[default]
    Dark,
    Light,
    CatppuccinLatte,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    CatppuccinMocha,
}

const THEME_CHOICES: [(&str, ThemeArg); 6] = [
    ("dark", ThemeArg::Dark),
    ("light", ThemeArg::Light),
    ("catppuccin-latte", ThemeArg::CatppuccinLatte),
    ("catppuccin-frappe", ThemeArg::CatppuccinFrappe),
    ("catppuccin-macchiato", ThemeArg::CatppuccinMacchiato),
    ("catppuccin-mocha", ThemeArg::CatppuccinMocha),
];

/// CLI arguments parsed from command line
#[derive(Debug, Clone, Default)]
pub struct CliArgs {
    pub theme: Option<ThemeArg>,
    /// Output to stdout instead of clipboard when exporting
    pub output_to_stdout: bool,
    /// Skip checking for updates on startup
    pub no_update_check: bool,
    /// Commit/revision range to review
    pub revisions: Option<String>,
}

impl ThemeArg {
    fn choices() -> &'static [(&'static str, ThemeArg)] {
        &THEME_CHOICES
    }

    pub fn from_str(s: &str) -> Option<Self> {
        let normalized = s.trim().to_ascii_lowercase();
        Self::choices().iter().find_map(|(name, theme)| {
            if *name == normalized {
                Some(*theme)
            } else {
                None
            }
        })
    }

    fn valid_values_display() -> String {
        Self::choices()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Resolve a theme based on the CLI argument
pub fn resolve_theme(arg: ThemeArg) -> Theme {
    match arg {
        ThemeArg::Dark => Theme::dark(),
        ThemeArg::Light => Theme::light(),
        ThemeArg::CatppuccinLatte => Theme::catppuccin_latte(),
        ThemeArg::CatppuccinFrappe => Theme::catppuccin_frappe(),
        ThemeArg::CatppuccinMacchiato => Theme::catppuccin_macchiato(),
        ThemeArg::CatppuccinMocha => Theme::catppuccin_mocha(),
    }
}

pub fn resolve_theme_arg_with_config(
    cli_theme: Option<ThemeArg>,
    config_theme: Option<&str>,
) -> (ThemeArg, Vec<String>) {
    let mut warnings = Vec::new();

    if let Some(theme) = cli_theme {
        return (theme, warnings);
    }

    if let Some(config_theme) = config_theme {
        if let Some(theme) = ThemeArg::from_str(config_theme) {
            return (theme, warnings);
        }

        let valid_values = ThemeArg::valid_values_display();
        warnings.push(format!(
            "Warning: Unknown theme '{config_theme}' in config, using dark. Valid options: {valid_values}"
        ));
    }

    (ThemeArg::Dark, warnings)
}

pub fn resolve_theme_with_config(
    cli_theme: Option<ThemeArg>,
    config_theme: Option<&str>,
) -> (Theme, Vec<String>) {
    let (theme_arg, warnings) = resolve_theme_arg_with_config(cli_theme, config_theme);
    (resolve_theme(theme_arg), warnings)
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
    let valid_values = ThemeArg::valid_values_display();
    let config_path = config_path_hint();
    println!(
        "tuicr - Review AI-generated diffs like a GitHub pull request

Usage: {name} [OPTIONS]

Options:
  -r, --revisions <REVSET>  Commit range/Revset to review (syntax depends on VCS backend)
  --theme <THEME>        Color theme to use [default: dark]
                         Valid values: {valid_values}
                         Precedence: --theme > {config_path} > dark
  --stdout               Output to stdout instead of clipboard when exporting
  --no-update-check      Skip checking for updates on startup
  -h, --help             Print this help message

Press ? in the application for keybinding help."
    );
    std::process::exit(0);
}

/// Parse CLI arguments from command line
///
/// We use a handrolled argument parser instead of clap to keep binary size
/// small and build times fast. If we end up needing more complex argument
/// handling, we can revisit this decision.
pub fn parse_cli_args() -> CliArgs {
    let args: Vec<String> = std::env::args().collect();
    parse_cli_args_from(&args).unwrap_or_else(|err| {
        eprintln!("Error: {err}");
        std::process::exit(2);
    })
}

fn parse_cli_args_from(args: &[String]) -> Result<CliArgs, String> {
    let mut cli_args = CliArgs::default();

    for i in 0..args.len() {
        // Handle --help / -h
        if args[i] == "--help" || args[i] == "-h" {
            print_help();
        }

        // Handle --stdout
        if args[i] == "--stdout" {
            cli_args.output_to_stdout = true;
        }

        // Handle --no-update-check
        if args[i] == "--no-update-check" {
            cli_args.no_update_check = true;
        }

        // Handle --theme value
        if args[i] == "--theme" {
            let valid_values = ThemeArg::valid_values_display();
            let value = args
                .get(i + 1)
                .ok_or_else(|| format!("--theme requires a value ({valid_values})"))?;

            if value.starts_with('-') {
                return Err(format!("--theme requires a value ({valid_values})"));
            }

            cli_args.theme = ThemeArg::from_str(value)
                .ok_or_else(|| format!("Unknown theme '{value}'. Valid options: {valid_values}"))
                .map(Some)?;
        }
        // Handle --theme=value
        if let Some(value) = args[i].strip_prefix("--theme=") {
            let valid_values = ThemeArg::valid_values_display();
            if value.is_empty() {
                return Err(format!("--theme requires a value ({valid_values})"));
            }

            cli_args.theme = ThemeArg::from_str(value)
                .ok_or_else(|| format!("Unknown theme '{value}'. Valid options: {valid_values}"))
                .map(Some)?;
        }

        // Handle -r / --revisions value
        if args[i] == "-r" || args[i] == "--revisions" {
            if let Some(value) = args.get(i + 1) {
                cli_args.revisions = Some(value.clone());
            } else {
                eprintln!("Warning: {0} requires a value", args[i]);
            }
        }
        // Handle --revisions=value
        if let Some(value) = args[i].strip_prefix("--revisions=") {
            cli_args.revisions = Some(value.to_string());
        }
    }

    Ok(cli_args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn parse_for_test(args: &[&str]) -> Result<CliArgs, String> {
        let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        parse_cli_args_from(&args)
    }

    #[test]
    fn should_parse_theme_when_provided() {
        let parsed = parse_for_test(&["tuicr", "--theme", "light"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some(ThemeArg::Light));
    }

    #[test]
    fn should_parse_catppuccin_themes() {
        let parsed = parse_for_test(&["tuicr", "--theme", "catppuccin-mocha"])
            .expect("parse should succeed");
        assert_eq!(parsed.theme, Some(ThemeArg::CatppuccinMocha));

        let parsed =
            parse_for_test(&["tuicr", "--theme=catppuccin-latte"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some(ThemeArg::CatppuccinLatte));
    }

    #[test]
    fn should_leave_theme_none_when_not_provided() {
        let parsed = parse_for_test(&["tuicr"]).expect("parse should succeed");
        assert_eq!(parsed.theme, None);
    }

    #[test]
    fn should_error_for_invalid_theme_in_separate_arg() {
        let err = parse_for_test(&["tuicr", "--theme", "nope"]).expect_err("parse should fail");
        assert!(err.contains("Unknown theme 'nope'"));
    }

    #[test]
    fn should_error_for_invalid_theme_in_equals_arg() {
        let err = parse_for_test(&["tuicr", "--theme=nope"]).expect_err("parse should fail");
        assert!(err.contains("Unknown theme 'nope'"));
    }

    #[test]
    fn should_error_when_theme_value_missing() {
        let err = parse_for_test(&["tuicr", "--theme"]).expect_err("parse should fail");
        assert!(err.contains("--theme requires a value"));
    }

    #[test]
    fn should_roundtrip_all_canonical_theme_values() {
        for (name, expected_theme) in ThemeArg::choices() {
            assert_eq!(ThemeArg::from_str(name), Some(*expected_theme));
        }
    }

    #[test]
    fn should_have_unique_theme_names_and_variants() {
        let names: HashSet<&str> = ThemeArg::choices().iter().map(|(name, _)| *name).collect();
        let variants: HashSet<ThemeArg> = ThemeArg::choices().iter().map(|(_, t)| *t).collect();
        assert_eq!(names.len(), ThemeArg::choices().len());
        assert_eq!(variants.len(), ThemeArg::choices().len());
    }

    #[test]
    fn should_use_cli_theme_over_config_theme() {
        let (resolved, warnings) =
            resolve_theme_arg_with_config(Some(ThemeArg::Light), Some("dark"));
        assert_eq!(resolved, ThemeArg::Light);
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_use_config_theme_when_cli_missing() {
        let (resolved, warnings) = resolve_theme_arg_with_config(None, Some("light"));
        assert_eq!(resolved, ThemeArg::Light);
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_fallback_to_dark_and_warn_for_invalid_config_theme() {
        let (resolved, warnings) = resolve_theme_arg_with_config(None, Some("unknown"));
        assert_eq!(resolved, ThemeArg::Dark);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Unknown theme 'unknown'"));
    }

    #[test]
    fn should_fallback_to_dark_when_no_theme_is_set() {
        let (resolved, warnings) = resolve_theme_arg_with_config(None, None);
        assert_eq!(resolved, ThemeArg::Dark);
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_use_catppuccin_theme_from_config_when_cli_missing() {
        let (resolved, warnings) = resolve_theme_arg_with_config(None, Some("catppuccin-fRappe"));
        assert_eq!(resolved, ThemeArg::CatppuccinFrappe);
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_resolve_catppuccin_mocha_syntect_theme() {
        let theme = resolve_theme(ThemeArg::CatppuccinMocha);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::CatppuccinMocha);
    }

    #[test]
    fn should_resolve_catppuccin_latte_syntect_theme() {
        let theme = resolve_theme(ThemeArg::CatppuccinLatte);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::CatppuccinLatte);
    }

    #[test]
    fn should_use_dark_flavor_base_for_catppuccin_mode_foreground() {
        let theme = Theme::catppuccin_mocha();
        assert_eq!(theme.mode_fg, Color::Rgb(30, 30, 46));
    }

    #[test]
    fn should_use_light_flavor_crust_for_catppuccin_mode_foreground() {
        let theme = Theme::catppuccin_latte();
        assert_eq!(theme.mode_fg, Color::Rgb(220, 224, 232));
    }

    #[test]
    fn should_blend_to_base_at_zero_percent() {
        let base = Color::Rgb(10, 20, 30);
        let accent = Color::Rgb(200, 210, 220);
        assert_eq!(blend(base, accent, 0), base);
    }

    #[test]
    fn should_blend_to_accent_at_hundred_percent() {
        let base = Color::Rgb(10, 20, 30);
        let accent = Color::Rgb(200, 210, 220);
        assert_eq!(blend(base, accent, 100), accent);
    }

    #[test]
    fn should_blend_midpoint_with_integer_rounding() {
        let base = Color::Rgb(0, 10, 20);
        let accent = Color::Rgb(100, 110, 120);
        assert_eq!(blend(base, accent, 50), Color::Rgb(50, 60, 70));
    }

    #[test]
    fn should_return_accent_for_non_rgb_blend_inputs() {
        let accent = Color::Rgb(100, 110, 120);
        assert_eq!(blend(Color::Reset, accent, 50), accent);
    }
}
