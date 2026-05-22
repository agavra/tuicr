//! Theme support for tuicr
//!
//! Provides dark and light themes with automatic terminal background detection.

use std::{
    fs,
    path::{Component, Path},
    process::Command,
    sync::OnceLock,
};

use ratatui::style::Color;
use syntect::highlighting::ThemeSet;
use two_face::theme::EmbeddedThemeName;

use crate::config::themes_dir;
use crate::syntax::SyntaxHighlighter;

#[derive(Clone)]
pub enum SyntaxThemeSource {
    Embedded(EmbeddedThemeName),
    Custom(Box<syntect::highlighting::Theme>),
}

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

    // Syntax highlighting source. Bundled themes use an embedded theme;
    // local themes may preload a custom `.tmTheme`.
    pub syntax_theme: SyntaxThemeSource,

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
    pub cursor_line_bg: Color,
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
            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::Base16EightiesDark),

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
            cursor_line_bg: Color::Rgb(40, 40, 45),
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
            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::Base16OceanLight),

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
            cursor_line_bg: Color::Rgb(225, 225, 235),
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

    pub fn solarized_light() -> Self {
        // Solarized palette
        let base03 = Color::Rgb(0, 43, 54);
        let base01 = Color::Rgb(88, 110, 117);
        let base00 = Color::Rgb(101, 123, 131);
        let base1 = Color::Rgb(147, 161, 161);
        let base2 = Color::Rgb(238, 232, 213);
        let base3 = Color::Rgb(253, 246, 227);
        let yellow = Color::Rgb(181, 137, 0);
        let orange = Color::Rgb(203, 75, 22);
        let red = Color::Rgb(220, 50, 47);
        let violet = Color::Rgb(108, 113, 196);
        let blue = Color::Rgb(38, 139, 210);
        let cyan = Color::Rgb(42, 161, 152);
        let green = Color::Rgb(133, 153, 0);

        Self {
            highlighter: OnceLock::new(),

            panel_bg: base3,
            bg_highlight: base2,
            fg_primary: base00,
            fg_secondary: base01,
            fg_dim: base1,

            diff_add: Color::Rgb(0, 80, 0),
            diff_add_bg: Color::Rgb(222, 240, 205),
            diff_del: Color::Rgb(140, 0, 0),
            diff_del_bg: Color::Rgb(252, 225, 224),
            diff_context: base00,
            diff_hunk_header: blue,
            expanded_context_fg: base1,

            syntax_add_bg: Color::Rgb(222, 240, 205),
            syntax_del_bg: Color::Rgb(252, 225, 224),

            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::SolarizedLight),

            file_added: green,
            file_modified: yellow,
            file_deleted: red,
            file_renamed: violet,

            reviewed: green,
            pending: yellow,

            comment_note: blue,
            comment_suggestion: cyan,
            comment_issue: red,
            comment_praise: green,

            border_focused: blue,
            border_unfocused: base1,
            status_bar_bg: base2,
            cursor_color: orange,
            cursor_line_bg: Color::Rgb(225, 222, 200),
            branch_name: cyan,
            help_indicator: base01,

            message_info_fg: base3,
            message_info_bg: blue,
            message_warning_fg: base03,
            message_warning_bg: yellow,
            message_error_fg: base3,
            message_error_bg: red,
            update_badge_fg: base03,
            update_badge_bg: yellow,

            mode_fg: base3,
            mode_bg: blue,
        }
    }

    pub fn solarized_dark() -> Self {
        let base03 = Color::Rgb(0, 43, 54);
        let base02 = Color::Rgb(7, 54, 66);
        let base01 = Color::Rgb(88, 110, 117);
        let base00 = Color::Rgb(101, 123, 131);
        let base0 = Color::Rgb(131, 148, 150);
        let base3 = Color::Rgb(253, 246, 227);
        let yellow = Color::Rgb(181, 137, 0);
        let orange = Color::Rgb(203, 75, 22);
        let red = Color::Rgb(220, 50, 47);
        let violet = Color::Rgb(108, 113, 196);
        let blue = Color::Rgb(38, 139, 210);
        let cyan = Color::Rgb(42, 161, 152);
        let green = Color::Rgb(133, 153, 0);

        Self {
            highlighter: OnceLock::new(),

            panel_bg: base03,
            bg_highlight: base02,
            fg_primary: base0,
            fg_secondary: base00,
            fg_dim: base01,

            diff_add: Color::Rgb(80, 220, 120),
            diff_add_bg: Color::Rgb(0, 60, 20),
            diff_del: Color::Rgb(240, 90, 90),
            diff_del_bg: Color::Rgb(70, 0, 0),
            diff_context: base0,
            diff_hunk_header: blue,
            expanded_context_fg: base01,

            syntax_add_bg: Color::Rgb(0, 60, 20),
            syntax_del_bg: Color::Rgb(70, 0, 0),

            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::SolarizedDark),

            file_added: green,
            file_modified: yellow,
            file_deleted: red,
            file_renamed: violet,

            reviewed: green,
            pending: yellow,

            comment_note: blue,
            comment_suggestion: cyan,
            comment_issue: red,
            comment_praise: green,

            border_focused: blue,
            border_unfocused: base01,
            status_bar_bg: base02,
            cursor_color: orange,
            cursor_line_bg: Color::Rgb(15, 60, 75),
            branch_name: cyan,
            help_indicator: base00,

            message_info_fg: base03,
            message_info_bg: blue,
            message_warning_fg: base03,
            message_warning_bg: yellow,
            message_error_fg: base3,
            message_error_bg: red,
            update_badge_fg: base03,
            update_badge_bg: yellow,

            mode_fg: base3,
            mode_bg: blue,
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

    pub fn ayu_light() -> Self {
        Self {
            highlighter: OnceLock::new(),

            // Base colors
            panel_bg: Color::Rgb(250, 250, 250),
            bg_highlight: Color::Rgb(240, 238, 228),
            fg_primary: Color::Rgb(92, 103, 115),
            fg_secondary: Color::Rgb(107, 118, 130),
            fg_dim: Color::Rgb(171, 176, 182),

            // Diff colors
            diff_add: Color::Rgb(134, 179, 0),
            diff_add_bg: Color::Rgb(238, 247, 208),
            diff_del: Color::Rgb(240, 113, 120),
            diff_del_bg: Color::Rgb(253, 235, 236),
            diff_context: Color::Rgb(92, 103, 115),
            diff_hunk_header: Color::Rgb(54, 163, 217),
            expanded_context_fg: Color::Rgb(130, 140, 153),

            // Syntax highlighting diff backgrounds
            syntax_add_bg: Color::Rgb(244, 251, 228),
            syntax_del_bg: Color::Rgb(255, 241, 242),

            // Syntect theme for syntax highlighting
            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::OneHalfLight),

            // File status colors
            file_added: Color::Rgb(134, 179, 0),
            file_modified: Color::Rgb(231, 197, 71),
            file_deleted: Color::Rgb(240, 113, 120),
            file_renamed: Color::Rgb(163, 122, 204),

            // Review status colors
            reviewed: Color::Rgb(134, 179, 0),
            pending: Color::Rgb(231, 197, 71),

            // Comment type colors
            comment_note: Color::Rgb(54, 163, 217),
            comment_suggestion: Color::Rgb(76, 191, 153),
            comment_issue: Color::Rgb(240, 113, 120),
            comment_praise: Color::Rgb(134, 179, 0),

            // UI element colors
            border_focused: Color::Rgb(54, 163, 217),
            border_unfocused: Color::Rgb(217, 216, 215),
            status_bar_bg: Color::Rgb(255, 255, 255),
            cursor_color: Color::Rgb(255, 106, 0),
            cursor_line_bg: Color::Rgb(235, 237, 240),
            branch_name: Color::Rgb(54, 163, 217),
            help_indicator: Color::Rgb(171, 176, 182),

            // Message/update badge colors
            message_info_fg: Color::Black,
            message_info_bg: Color::Rgb(140, 220, 255),
            message_warning_fg: Color::Black,
            message_warning_bg: Color::Rgb(246, 217, 140),
            message_error_fg: Color::White,
            message_error_bg: Color::Rgb(217, 87, 87),
            update_badge_fg: Color::Black,
            update_badge_bg: Color::Rgb(246, 217, 140),

            // Mode indicator colors
            mode_fg: Color::White,
            mode_bg: Color::Rgb(255, 106, 0),
        }
    }

    /// Ayu Mirage — the dark variant of the Ayu palette
    /// (<https://ayutheme.com/>). Hex values are the resolved outputs
    /// of the official `ayu-colors` palette generator, matching the
    /// upstream `vscode-ayu` extension.
    pub fn ayu_mirage() -> Self {
        let bg = Color::Rgb(31, 36, 48); // #1f2430 surface.base / ui.bg
        let bg_dark = Color::Rgb(26, 31, 41); // #1a1f29 editor.line
        let bg_panel = Color::Rgb(40, 46, 59); // #282e3b ui.panel.bg
        let selection = Color::Rgb(41, 48, 64); // #293040 ui.selection.active on bg
        let fg = Color::Rgb(204, 202, 194); // #cccac2 editor.fg
        let fg_secondary = Color::Rgb(154, 162, 175);
        let comment = Color::Rgb(110, 124, 143); // #6e7c8f syntax.comment
        let dim = Color::Rgb(112, 122, 140); // #707a8c ui.fg

        let yellow = Color::Rgb(255, 205, 102); // #ffcd66 syntax.func
        let orange = Color::Rgb(255, 166, 89); // #ffa659 syntax.keyword
        let green = Color::Rgb(135, 217, 108); // #87d96c vcs.added
        let red = Color::Rgb(242, 121, 131); // #f27983 vcs.removed
        let cyan = Color::Rgb(92, 207, 230); // #5ccfe6 syntax.tag
        let blue = Color::Rgb(115, 208, 255); // #73d0ff syntax.entity
        let purple = Color::Rgb(223, 191, 255); // #dfbfff syntax.constant
        let mint = Color::Rgb(149, 230, 203); // #95e6cb syntax.regexp

        Self {
            highlighter: OnceLock::new(),

            panel_bg: bg,
            bg_highlight: selection,
            fg_primary: fg,
            fg_secondary,
            fg_dim: comment,

            diff_add: green,
            diff_add_bg: Color::Rgb(35, 53, 41),
            diff_del: red,
            diff_del_bg: Color::Rgb(58, 36, 41),
            diff_context: fg,
            diff_hunk_header: blue,
            expanded_context_fg: dim,

            syntax_add_bg: Color::Rgb(30, 47, 36),
            syntax_del_bg: Color::Rgb(50, 30, 35),

            // Closest embedded base16 dark to the Ayu Mirage feel.
            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::Base16EightiesDark),

            file_added: green,
            file_modified: yellow,
            file_deleted: red,
            file_renamed: purple,

            reviewed: green,
            pending: yellow,

            comment_note: blue,
            comment_suggestion: mint,
            comment_issue: red,
            comment_praise: green,

            border_focused: orange,
            border_unfocused: Color::Rgb(63, 70, 84),
            status_bar_bg: bg_dark,
            cursor_color: orange,
            cursor_line_bg: bg_panel,
            branch_name: cyan,
            help_indicator: comment,

            message_info_fg: bg_dark,
            message_info_bg: blue,
            message_warning_fg: bg_dark,
            message_warning_bg: yellow,
            message_error_fg: fg,
            message_error_bg: red,
            update_badge_fg: bg_dark,
            update_badge_bg: yellow,

            mode_fg: bg_dark,
            mode_bg: orange,
        }
    }

    pub fn onedark() -> Self {
        Self {
            highlighter: OnceLock::new(),

            // Base colors
            panel_bg: Color::Rgb(40, 44, 52),
            bg_highlight: Color::Rgb(62, 68, 82),
            fg_primary: Color::Rgb(171, 178, 191),
            fg_secondary: Color::Rgb(192, 198, 208),
            fg_dim: Color::Rgb(92, 99, 112),

            // Diff colors
            diff_add: Color::Rgb(152, 195, 121),
            diff_add_bg: Color::Rgb(44, 56, 43),
            diff_del: Color::Rgb(224, 108, 117),
            diff_del_bg: Color::Rgb(58, 45, 47),
            diff_context: Color::Rgb(171, 178, 191),
            diff_hunk_header: Color::Rgb(86, 182, 194),
            expanded_context_fg: Color::Rgb(92, 99, 112),

            // Syntax highlighting diff backgrounds
            syntax_add_bg: Color::Rgb(37, 49, 38),
            syntax_del_bg: Color::Rgb(59, 37, 40),

            // Syntect theme for syntax highlighting
            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::OneHalfDark),

            // File status colors
            file_added: Color::Rgb(152, 195, 121),
            file_modified: Color::Rgb(229, 192, 123),
            file_deleted: Color::Rgb(224, 108, 117),
            file_renamed: Color::Rgb(198, 120, 221),

            // Review status colors
            reviewed: Color::Rgb(152, 195, 121),
            pending: Color::Rgb(229, 192, 123),

            // Comment type colors
            comment_note: Color::Rgb(97, 175, 239),
            comment_suggestion: Color::Rgb(86, 182, 194),
            comment_issue: Color::Rgb(224, 108, 117),
            comment_praise: Color::Rgb(152, 195, 121),

            // UI element colors
            border_focused: Color::Rgb(97, 175, 239),
            border_unfocused: Color::Rgb(62, 68, 82),
            status_bar_bg: Color::Rgb(33, 37, 43),
            cursor_color: Color::Rgb(229, 192, 123),
            cursor_line_bg: Color::Rgb(44, 49, 58),
            branch_name: Color::Rgb(86, 182, 194),
            help_indicator: Color::Rgb(92, 99, 112),

            // Message/update badge colors
            message_info_fg: Color::Black,
            message_info_bg: Color::Rgb(86, 182, 194),
            message_warning_fg: Color::Black,
            message_warning_bg: Color::Rgb(229, 192, 123),
            message_error_fg: Color::White,
            message_error_bg: Color::Rgb(224, 108, 117),
            update_badge_fg: Color::Black,
            update_badge_bg: Color::Rgb(229, 192, 123),

            // Mode indicator colors
            mode_fg: Color::Rgb(40, 44, 52),
            mode_bg: Color::Rgb(97, 175, 239),
        }
    }

    /// GitHub light theme (matches the github.com diff palette: Primer light tokens)
    pub fn github_light() -> Self {
        Self {
            highlighter: OnceLock::new(),

            panel_bg: Color::Rgb(255, 255, 255),
            bg_highlight: Color::Rgb(221, 244, 255),
            fg_primary: Color::Rgb(31, 35, 40),
            fg_secondary: Color::Rgb(89, 99, 110),
            fg_dim: Color::Rgb(110, 119, 129),

            diff_add: Color::Rgb(26, 127, 55),
            diff_add_bg: Color::Rgb(230, 255, 236),
            diff_del: Color::Rgb(207, 34, 46),
            diff_del_bg: Color::Rgb(255, 235, 233),
            diff_context: Color::Rgb(31, 35, 40),
            diff_hunk_header: Color::Rgb(9, 105, 218),
            expanded_context_fg: Color::Rgb(110, 119, 129),

            syntax_add_bg: Color::Rgb(230, 255, 236),
            syntax_del_bg: Color::Rgb(255, 235, 233),

            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::InspiredGithub),

            file_added: Color::Rgb(26, 127, 55),
            file_modified: Color::Rgb(154, 103, 0),
            file_deleted: Color::Rgb(207, 34, 46),
            file_renamed: Color::Rgb(130, 80, 223),

            reviewed: Color::Rgb(26, 127, 55),
            pending: Color::Rgb(154, 103, 0),

            comment_note: Color::Rgb(9, 105, 218),
            comment_suggestion: Color::Rgb(20, 130, 130),
            comment_issue: Color::Rgb(207, 34, 46),
            comment_praise: Color::Rgb(26, 127, 55),

            border_focused: Color::Rgb(9, 105, 218),
            border_unfocused: Color::Rgb(208, 215, 222),
            status_bar_bg: Color::Rgb(246, 248, 250),
            cursor_color: Color::Rgb(154, 103, 0),
            cursor_line_bg: Color::Rgb(221, 244, 255),
            branch_name: Color::Rgb(9, 105, 218),
            help_indicator: Color::Rgb(110, 119, 129),

            message_info_fg: Color::White,
            message_info_bg: Color::Rgb(9, 105, 218),
            message_warning_fg: Color::White,
            message_warning_bg: Color::Rgb(154, 103, 0),
            message_error_fg: Color::White,
            message_error_bg: Color::Rgb(207, 34, 46),
            update_badge_fg: Color::White,
            update_badge_bg: Color::Rgb(154, 103, 0),

            mode_fg: Color::White,
            mode_bg: Color::Rgb(9, 105, 218),
        }
    }

    /// GitHub dark theme (matches the github.com dark mode diff palette: Primer dark tokens)
    pub fn github_dark() -> Self {
        Self {
            highlighter: OnceLock::new(),

            panel_bg: Color::Rgb(13, 17, 23),
            bg_highlight: Color::Rgb(33, 38, 45),
            fg_primary: Color::Rgb(230, 237, 243),
            fg_secondary: Color::Rgb(201, 209, 217),
            fg_dim: Color::Rgb(139, 148, 158),

            diff_add: Color::Rgb(63, 185, 80),
            diff_add_bg: Color::Rgb(16, 35, 28),
            diff_del: Color::Rgb(248, 81, 73),
            diff_del_bg: Color::Rgb(48, 27, 31),
            diff_context: Color::Rgb(230, 237, 243),
            diff_hunk_header: Color::Rgb(88, 166, 255),
            expanded_context_fg: Color::Rgb(139, 148, 158),

            syntax_add_bg: Color::Rgb(16, 35, 28),
            syntax_del_bg: Color::Rgb(48, 27, 31),

            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::OneHalfDark),

            file_added: Color::Rgb(63, 185, 80),
            file_modified: Color::Rgb(210, 153, 34),
            file_deleted: Color::Rgb(248, 81, 73),
            file_renamed: Color::Rgb(163, 113, 247),

            reviewed: Color::Rgb(63, 185, 80),
            pending: Color::Rgb(210, 153, 34),

            comment_note: Color::Rgb(88, 166, 255),
            comment_suggestion: Color::Rgb(86, 212, 221),
            comment_issue: Color::Rgb(248, 81, 73),
            comment_praise: Color::Rgb(63, 185, 80),

            border_focused: Color::Rgb(88, 166, 255),
            border_unfocused: Color::Rgb(48, 54, 61),
            status_bar_bg: Color::Rgb(22, 27, 34),
            cursor_color: Color::Rgb(210, 153, 34),
            cursor_line_bg: Color::Rgb(22, 27, 34),
            branch_name: Color::Rgb(88, 166, 255),
            help_indicator: Color::Rgb(139, 148, 158),

            message_info_fg: Color::Rgb(13, 17, 23),
            message_info_bg: Color::Rgb(88, 166, 255),
            message_warning_fg: Color::Rgb(13, 17, 23),
            message_warning_bg: Color::Rgb(210, 153, 34),
            message_error_fg: Color::White,
            message_error_bg: Color::Rgb(248, 81, 73),
            update_badge_fg: Color::Rgb(13, 17, 23),
            update_badge_bg: Color::Rgb(210, 153, 34),

            mode_fg: Color::White,
            mode_bg: Color::Rgb(88, 166, 255),
        }
    }

    /// Tokyo Night Storm (folke/tokyonight.nvim "storm" variant)
    pub fn tokyo_night_storm() -> Self {
        let bg = Color::Rgb(36, 40, 59); // #24283b
        let bg_dark = Color::Rgb(31, 35, 53); // #1f2335
        let bg_highlight = Color::Rgb(41, 46, 66); // #292e42
        let terminal_black = Color::Rgb(65, 72, 104); // #414868
        let fg = Color::Rgb(192, 202, 245); // #c0caf5
        let fg_dark = Color::Rgb(169, 177, 214); // #a9b1d6
        let dark3 = Color::Rgb(84, 92, 126); // #545c7e
        let comment = Color::Rgb(86, 95, 137); // #565f89
        let blue = Color::Rgb(122, 162, 247); // #7aa2f7
        let cyan = Color::Rgb(125, 207, 255); // #7dcfff
        let magenta = Color::Rgb(187, 154, 247); // #bb9af7
        let orange = Color::Rgb(255, 158, 100); // #ff9e64
        let yellow = Color::Rgb(224, 175, 104); // #e0af68
        let green = Color::Rgb(158, 206, 106); // #9ece6a
        let red = Color::Rgb(247, 118, 142); // #f7768e

        Self {
            highlighter: OnceLock::new(),

            panel_bg: bg,
            bg_highlight,
            fg_primary: fg,
            fg_secondary: fg_dark,
            fg_dim: dark3,

            diff_add: green,
            diff_add_bg: Color::Rgb(32, 48, 59), // #20303b
            diff_del: red,
            diff_del_bg: Color::Rgb(55, 34, 44), // #37222c
            diff_context: fg_dark,
            diff_hunk_header: blue,
            expanded_context_fg: dark3,

            syntax_add_bg: Color::Rgb(28, 42, 52),
            syntax_del_bg: Color::Rgb(47, 30, 38),

            // Closest embedded base16 dark with the muted blue/purple feel
            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::Base16EightiesDark),

            file_added: green,
            file_modified: yellow,
            file_deleted: red,
            file_renamed: magenta,

            reviewed: green,
            pending: yellow,

            comment_note: blue,
            comment_suggestion: cyan,
            comment_issue: red,
            comment_praise: green,

            border_focused: blue,
            border_unfocused: terminal_black,
            status_bar_bg: bg_dark,
            cursor_color: orange,
            cursor_line_bg: bg_highlight,
            branch_name: cyan,
            help_indicator: comment,

            message_info_fg: bg_dark,
            message_info_bg: blue,
            message_warning_fg: bg_dark,
            message_warning_bg: yellow,
            message_error_fg: fg,
            message_error_bg: red,
            update_badge_fg: bg_dark,
            update_badge_bg: yellow,

            mode_fg: bg_dark,
            mode_bg: blue,
        }
    }

    /// Tokyo Night Day (folke/tokyonight.nvim "day" variant)
    pub fn tokyo_night_day() -> Self {
        let bg = Color::Rgb(225, 226, 231); // #e1e2e7
        let bg_dark = Color::Rgb(208, 213, 227); // #d0d5e3
        let bg_highlight = Color::Rgb(196, 200, 218); // #c4c8da
        let terminal_black = Color::Rgb(108, 110, 117); // #6c6e75
        let fg = Color::Rgb(55, 96, 191); // #3760bf
        let fg_dark = Color::Rgb(97, 114, 176); // #6172b0
        let dark3 = Color::Rgb(132, 140, 181); // #848cb5
        let comment = Color::Rgb(132, 140, 181); // #848cb5
        let blue = Color::Rgb(46, 125, 233); // #2e7de9
        let cyan = Color::Rgb(0, 113, 151); // #007197
        let magenta = Color::Rgb(120, 71, 189); // #7847bd
        let orange = Color::Rgb(177, 92, 0); // #b15c00
        let yellow = Color::Rgb(140, 108, 62); // #8c6c3e
        let green = Color::Rgb(88, 117, 57); // #587539
        let red = Color::Rgb(245, 42, 101); // #f52a65

        Self {
            highlighter: OnceLock::new(),

            panel_bg: bg,
            bg_highlight,
            fg_primary: fg,
            fg_secondary: fg_dark,
            fg_dim: dark3,

            diff_add: green,
            diff_add_bg: Color::Rgb(197, 221, 230), // #c5dde6 (light blue-tinted)
            diff_del: red,
            diff_del_bg: Color::Rgb(243, 197, 203), // #f3c5cb (light rose)
            diff_context: fg,
            diff_hunk_header: blue,
            expanded_context_fg: dark3,

            syntax_add_bg: Color::Rgb(216, 230, 236), // #d8e6ec
            syntax_del_bg: Color::Rgb(245, 213, 217), // #f5d5d9

            // Light syntect base with muted blue feel, matching the storm sibling's choice of a base16 family
            syntax_theme: SyntaxThemeSource::Embedded(EmbeddedThemeName::Base16OceanLight),

            file_added: green,
            file_modified: yellow,
            file_deleted: red,
            file_renamed: magenta,

            reviewed: green,
            pending: yellow,

            comment_note: blue,
            comment_suggestion: cyan,
            comment_issue: red,
            comment_praise: green,

            border_focused: blue,
            border_unfocused: terminal_black,
            status_bar_bg: bg_dark,
            cursor_color: orange,
            cursor_line_bg: bg_highlight,
            branch_name: cyan,
            help_indicator: comment,

            message_info_fg: bg,
            message_info_bg: blue,
            message_warning_fg: bg,
            message_warning_bg: yellow,
            message_error_fg: bg,
            message_error_bg: red,
            update_badge_fg: bg,
            update_badge_bg: yellow,

            mode_fg: bg,
            mode_bg: blue,
        }
    }

    pub fn gruvbox_dark() -> Self {
        let flavor = GruvboxFlavor {
            dark: true,
            bg0: rgb(29, 32, 33),
            bg1: rgb(40, 40, 40),
            bg4: rgb(80, 73, 69),
            selected_bg: rgb(60, 56, 54),
            fg0: rgb(212, 190, 152),
            fg1: rgb(221, 199, 161),
            grey0: rgb(124, 111, 100),
            grey1: rgb(146, 131, 116),
            red: rgb(251, 73, 52),
            orange: rgb(254, 128, 25),
            yellow: rgb(250, 189, 47),
            green: rgb(184, 187, 38),
            aqua: rgb(142, 192, 124),
            blue: rgb(131, 165, 152),
            purple: rgb(211, 134, 155),
            bg_red: rgb(64, 33, 32),
            bg_green: rgb(52, 56, 27),
        };
        gruvbox_theme(flavor)
    }

    pub fn gruvbox_light() -> Self {
        let flavor = GruvboxFlavor {
            dark: false,
            bg0: rgb(249, 245, 215),
            bg1: rgb(245, 237, 202),
            bg4: rgb(221, 199, 161),
            selected_bg: rgb(235, 219, 178),
            fg0: rgb(101, 71, 53),
            fg1: rgb(79, 56, 41),
            grey0: rgb(168, 153, 132),
            grey1: rgb(146, 131, 116),
            red: rgb(157, 0, 6),
            orange: rgb(175, 58, 3),
            yellow: rgb(181, 118, 20),
            green: rgb(121, 116, 14),
            aqua: rgb(66, 123, 88),
            blue: rgb(7, 102, 120),
            purple: rgb(143, 63, 113),
            bg_red: rgb(240, 222, 222),
            bg_green: rgb(228, 236, 213),
        };
        gruvbox_theme(flavor)
    }

    pub fn nord_dark() -> Self {
        nord_theme(NordFlavor {
            dark: true,
            bg0: rgb(46, 52, 64),       // nord0
            bg1: rgb(59, 66, 82),       // nord1
            bg2: rgb(67, 76, 94),       // nord2
            bg3: rgb(76, 86, 106),      // nord3
            fg0: rgb(216, 222, 233),    // nord4
            fg1: rgb(229, 233, 240),    // nord5
            frost0: rgb(143, 188, 187), // nord7
            frost1: rgb(136, 192, 208), // nord8
            frost2: rgb(129, 161, 193), // nord9
            red: rgb(191, 97, 106),     // nord11
            orange: rgb(208, 135, 112), // nord12
            yellow: rgb(235, 203, 139), // nord13
            green: rgb(163, 190, 140),  // nord14
            syntect_theme: EmbeddedThemeName::Nord,
        })
    }

    pub fn nord_light() -> Self {
        nord_theme(NordFlavor {
            dark: false,
            bg0: rgb(236, 239, 244),    // nord6
            bg1: rgb(229, 233, 240),    // nord5
            bg2: rgb(216, 222, 233),    // nord4
            bg3: rgb(76, 86, 106),      // nord3
            fg0: rgb(46, 52, 64),       // nord0
            fg1: rgb(59, 66, 82),       // nord1
            frost0: rgb(143, 188, 187), // nord7
            frost1: rgb(136, 192, 208), // nord8
            frost2: rgb(129, 161, 193), // nord9
            red: rgb(191, 97, 106),     // nord11
            orange: rgb(208, 135, 112), // nord12
            yellow: rgb(235, 203, 139), // nord13
            green: rgb(163, 190, 140),  // nord14
            syntect_theme: EmbeddedThemeName::Base16OceanLight,
        })
    }

    pub fn nord_dark_high_contrast() -> Self {
        nord_theme(NordFlavor {
            dark: true,
            bg0: rgb(46, 52, 64),       // nord0
            bg1: rgb(59, 66, 82),       // nord1
            bg2: rgb(67, 76, 94),       // nord2
            bg3: rgb(76, 86, 106),      // nord3
            fg0: rgb(236, 239, 244),    // nord6 (boosted from nord4 for contrast)
            fg1: rgb(229, 233, 240),    // nord5
            frost0: rgb(143, 188, 187), // nord7
            frost1: rgb(136, 192, 208), // nord8
            frost2: rgb(129, 161, 193), // nord9
            red: rgb(191, 97, 106),     // nord11
            orange: rgb(208, 135, 112), // nord12
            yellow: rgb(235, 203, 139), // nord13
            green: rgb(163, 190, 140),  // nord14
            syntect_theme: EmbeddedThemeName::Nord,
        })
    }

    pub fn nord_light_high_contrast() -> Self {
        nord_theme(NordFlavor {
            dark: false,
            bg0: rgb(236, 239, 244),    // nord6
            bg1: rgb(229, 233, 240),    // nord5
            bg2: rgb(216, 222, 233),    // nord4
            bg3: rgb(67, 76, 94),       // nord2 (deeper than nord3 for contrast)
            fg0: rgb(46, 52, 64),       // nord0
            fg1: rgb(59, 66, 82),       // nord1
            frost0: rgb(143, 188, 187), // nord7
            frost1: rgb(136, 192, 208), // nord8
            frost2: rgb(129, 161, 193), // nord9
            red: rgb(191, 97, 106),     // nord11
            orange: rgb(208, 135, 112), // nord12
            yellow: rgb(235, 203, 139), // nord13
            green: rgb(163, 190, 140),  // nord14
            syntect_theme: EmbeddedThemeName::Base16OceanLight,
        })
    }

    /// Everforest dark (medium variant) — sainnhe/everforest palette.
    pub fn everforest_dark() -> Self {
        everforest_theme(EverforestFlavor {
            dark: true,
            bg0: rgb(45, 53, 59),       // #2d353b
            bg1: rgb(52, 63, 68),       // #343f44
            bg3: rgb(71, 82, 88),       // #475258
            bg5: rgb(86, 99, 95),       // #56635f
            bg_red: rgb(81, 64, 69),    // #514045
            bg_green: rgb(66, 80, 71),  // #425047
            fg: rgb(211, 198, 170),     // #d3c6aa
            grey0: rgb(122, 132, 120),  // #7a8478
            grey1: rgb(133, 146, 137),  // #859289
            grey2: rgb(157, 169, 160),  // #9da9a0
            red: rgb(230, 126, 128),    // #e67e80
            orange: rgb(230, 152, 117), // #e69875
            yellow: rgb(219, 188, 127), // #dbbc7f
            green: rgb(167, 192, 128),  // #a7c080
            aqua: rgb(131, 192, 146),   // #83c092
            blue: rgb(127, 187, 179),   // #7fbbb3
            purple: rgb(214, 153, 182), // #d699b6
            // two-face has no Everforest syntect theme; Gruvbox is the
            // closest warm, low-saturation, earthy match available.
            syntect_theme: EmbeddedThemeName::GruvboxDark,
        })
    }

    /// Everforest light (medium variant) — sainnhe/everforest palette.
    pub fn everforest_light() -> Self {
        everforest_theme(EverforestFlavor {
            dark: false,
            bg0: rgb(253, 246, 227),      // #fdf6e3
            bg1: rgb(244, 240, 217),      // #f4f0d9
            bg3: rgb(230, 226, 204),      // #e6e2cc
            bg5: rgb(189, 195, 175),      // #bdc3af
            bg_red: rgb(253, 227, 218),   // #fde3da
            bg_green: rgb(240, 241, 210), // #f0f1d2
            fg: rgb(92, 106, 114),        // #5c6a72
            grey0: rgb(166, 176, 160),    // #a6b0a0
            grey1: rgb(147, 159, 145),    // #939f91
            grey2: rgb(130, 145, 129),    // #829181
            red: rgb(248, 85, 82),        // #f85552
            orange: rgb(245, 125, 38),    // #f57d26
            yellow: rgb(223, 160, 0),     // #dfa000
            green: rgb(141, 161, 1),      // #8da101
            aqua: rgb(53, 167, 124),      // #35a77c
            blue: rgb(58, 148, 197),      // #3a94c5
            purple: rgb(223, 105, 186),   // #df69ba
            // two-face has no Everforest syntect theme; Gruvbox is the
            // closest warm, low-saturation, earthy match available.
            syntect_theme: EmbeddedThemeName::GruvboxLight,
        })
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

#[derive(Clone, Copy)]
struct NordFlavor {
    dark: bool,
    bg0: Color,
    bg1: Color,
    bg2: Color,
    bg3: Color,
    fg0: Color,
    fg1: Color,
    frost0: Color,
    frost1: Color,
    frost2: Color,
    red: Color,
    orange: Color,
    yellow: Color,
    green: Color,
    syntect_theme: EmbeddedThemeName,
}

#[derive(Clone, Copy)]
struct EverforestFlavor {
    dark: bool,
    bg0: Color,
    bg1: Color,
    bg3: Color,
    bg5: Color,
    bg_red: Color,
    bg_green: Color,
    fg: Color,
    grey0: Color,
    grey1: Color,
    grey2: Color,
    red: Color,
    orange: Color,
    yellow: Color,
    green: Color,
    aqua: Color,
    blue: Color,
    purple: Color,
    syntect_theme: EmbeddedThemeName,
}

#[derive(Clone, Copy)]
struct GruvboxFlavor {
    dark: bool,
    bg0: Color,
    bg1: Color,
    bg4: Color,
    selected_bg: Color,
    fg0: Color,
    fg1: Color,
    grey0: Color,
    grey1: Color,
    red: Color,
    orange: Color,
    yellow: Color,
    green: Color,
    aqua: Color,
    blue: Color,
    purple: Color,
    bg_red: Color,
    bg_green: Color,
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
        syntax_theme: SyntaxThemeSource::Embedded(syntect_theme),

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
        cursor_line_bg: flavor.surface1,
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

fn gruvbox_theme(flavor: GruvboxFlavor) -> Theme {
    let syntect_theme = if flavor.dark {
        EmbeddedThemeName::GruvboxDark
    } else {
        EmbeddedThemeName::GruvboxLight
    };
    let accent_fg = if flavor.dark { flavor.bg0 } else { flavor.fg1 };

    Theme {
        highlighter: OnceLock::new(),

        // Base colors
        panel_bg: flavor.bg0,
        bg_highlight: flavor.selected_bg,
        fg_primary: flavor.fg0,
        fg_secondary: flavor.fg1,
        fg_dim: flavor.grey0,

        // Diff colors
        diff_add: flavor.green,
        diff_add_bg: flavor.bg_green,
        diff_del: flavor.red,
        diff_del_bg: flavor.bg_red,
        diff_context: flavor.fg0,
        diff_hunk_header: flavor.blue,
        expanded_context_fg: flavor.grey1,

        // Syntax highlighting diff backgrounds
        syntax_add_bg: flavor.bg_green,
        syntax_del_bg: flavor.bg_red,

        // Syntect theme for syntax highlighting
        syntax_theme: SyntaxThemeSource::Embedded(syntect_theme),

        // File status colors
        file_added: flavor.green,
        file_modified: flavor.yellow,
        file_deleted: flavor.red,
        file_renamed: flavor.purple,

        // Review status colors
        reviewed: flavor.green,
        pending: flavor.yellow,

        // Comment type colors
        comment_note: flavor.blue,
        comment_suggestion: flavor.aqua,
        comment_issue: flavor.red,
        comment_praise: flavor.green,

        // UI element colors
        border_focused: flavor.aqua,
        border_unfocused: flavor.bg4,
        status_bar_bg: flavor.bg1,
        cursor_color: flavor.orange,
        cursor_line_bg: flavor.selected_bg,
        branch_name: flavor.aqua,
        help_indicator: flavor.grey0,

        // Message/update badge colors
        message_info_fg: accent_fg,
        message_info_bg: flavor.aqua,
        message_warning_fg: accent_fg,
        message_warning_bg: flavor.yellow,
        message_error_fg: accent_fg,
        message_error_bg: flavor.red,
        update_badge_fg: accent_fg,
        update_badge_bg: flavor.orange,

        // Mode indicator colors
        mode_fg: accent_fg,
        mode_bg: flavor.green,
    }
}

fn everforest_theme(flavor: EverforestFlavor) -> Theme {
    let accent_fg = if flavor.dark { flavor.bg0 } else { flavor.fg };
    let syntax_add_bg = blend(flavor.bg0, flavor.green, 12);
    let syntax_del_bg = blend(flavor.bg0, flavor.red, 12);

    Theme {
        highlighter: OnceLock::new(),

        panel_bg: flavor.bg0,
        bg_highlight: flavor.bg3,
        fg_primary: flavor.fg,
        fg_secondary: flavor.grey2,
        fg_dim: flavor.grey0,

        diff_add: flavor.green,
        diff_add_bg: flavor.bg_green,
        diff_del: flavor.red,
        diff_del_bg: flavor.bg_red,
        diff_context: flavor.fg,
        diff_hunk_header: flavor.blue,
        expanded_context_fg: flavor.grey1,

        syntax_add_bg,
        syntax_del_bg,

        syntax_theme: SyntaxThemeSource::Embedded(flavor.syntect_theme),

        file_added: flavor.green,
        file_modified: flavor.yellow,
        file_deleted: flavor.red,
        file_renamed: flavor.purple,

        reviewed: flavor.green,
        pending: flavor.yellow,

        comment_note: flavor.blue,
        comment_suggestion: flavor.aqua,
        comment_issue: flavor.red,
        comment_praise: flavor.green,

        border_focused: flavor.aqua,
        border_unfocused: flavor.bg5,
        status_bar_bg: flavor.bg1,
        cursor_color: flavor.orange,
        cursor_line_bg: flavor.bg1,
        branch_name: flavor.aqua,
        help_indicator: flavor.grey0,

        message_info_fg: accent_fg,
        message_info_bg: flavor.blue,
        message_warning_fg: accent_fg,
        message_warning_bg: flavor.yellow,
        message_error_fg: accent_fg,
        message_error_bg: flavor.red,
        update_badge_fg: accent_fg,
        update_badge_bg: flavor.orange,

        mode_fg: accent_fg,
        mode_bg: flavor.green,
    }
}

fn nord_theme(flavor: NordFlavor) -> Theme {
    let accent_fg = if flavor.dark { flavor.bg0 } else { flavor.fg1 };
    let diff_add_bg = blend(flavor.bg0, flavor.green, 15);
    let diff_del_bg = blend(flavor.bg0, flavor.red, 15);
    let syntax_add_bg = blend(flavor.bg0, flavor.green, 10);
    let syntax_del_bg = blend(flavor.bg0, flavor.red, 10);

    Theme {
        highlighter: OnceLock::new(),

        panel_bg: flavor.bg0,
        bg_highlight: flavor.bg1,
        fg_primary: flavor.fg0,
        fg_secondary: flavor.fg1,
        fg_dim: flavor.bg3,

        diff_add: flavor.green,
        diff_add_bg,
        diff_del: flavor.red,
        diff_del_bg,
        diff_context: flavor.fg0,
        diff_hunk_header: flavor.frost1,
        expanded_context_fg: flavor.bg3,

        syntax_add_bg,
        syntax_del_bg,

        syntax_theme: SyntaxThemeSource::Embedded(flavor.syntect_theme),

        file_added: flavor.green,
        file_modified: flavor.yellow,
        file_deleted: flavor.red,
        file_renamed: flavor.frost2,

        reviewed: flavor.green,
        pending: flavor.yellow,

        comment_note: flavor.frost1,
        comment_suggestion: flavor.frost0,
        comment_issue: flavor.red,
        comment_praise: flavor.green,

        border_focused: flavor.frost1,
        border_unfocused: flavor.bg1,
        status_bar_bg: flavor.bg2,
        cursor_color: flavor.frost2,
        cursor_line_bg: flavor.bg2,
        branch_name: flavor.frost0,
        help_indicator: flavor.bg3,

        message_info_fg: accent_fg,
        message_info_bg: flavor.frost1,
        message_warning_fg: accent_fg,
        message_warning_bg: flavor.orange,
        message_error_fg: accent_fg,
        message_error_bg: flavor.red,
        update_badge_fg: accent_fg,
        update_badge_bg: flavor.orange,

        mode_fg: accent_fg,
        mode_bg: flavor.frost1,
    }
}

/// Theme selection from CLI argument
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeArg {
    #[default]
    Dark,
    Light,
    AyuLight,
    AyuMirage,
    Onedark,
    GithubLight,
    GithubDark,
    CatppuccinLatte,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    CatppuccinMocha,
    GruvboxDark,
    GruvboxLight,
    NordDark,
    NordLight,
    NordDarkHighContrast,
    NordLightHighContrast,
    SolarizedLight,
    SolarizedDark,
    TokyoNightStorm,
    TokyoNightDay,
    EverforestDark,
    EverforestLight,
}

const THEME_CHOICES: [(&str, ThemeArg); 23] = [
    ("dark", ThemeArg::Dark),
    ("light", ThemeArg::Light),
    ("ayu-light", ThemeArg::AyuLight),
    ("ayu-mirage", ThemeArg::AyuMirage),
    ("onedark", ThemeArg::Onedark),
    ("github-light", ThemeArg::GithubLight),
    ("github-dark", ThemeArg::GithubDark),
    ("catppuccin-latte", ThemeArg::CatppuccinLatte),
    ("catppuccin-frappe", ThemeArg::CatppuccinFrappe),
    ("catppuccin-macchiato", ThemeArg::CatppuccinMacchiato),
    ("catppuccin-mocha", ThemeArg::CatppuccinMocha),
    ("gruvbox-dark", ThemeArg::GruvboxDark),
    ("gruvbox-light", ThemeArg::GruvboxLight),
    ("nord-dark", ThemeArg::NordDark),
    ("nord-light", ThemeArg::NordLight),
    ("nord-dark-high-contrast", ThemeArg::NordDarkHighContrast),
    ("nord-light-high-contrast", ThemeArg::NordLightHighContrast),
    ("solarized-light", ThemeArg::SolarizedLight),
    ("solarized-dark", ThemeArg::SolarizedDark),
    ("tokyo-night-storm", ThemeArg::TokyoNightStorm),
    ("tokyo-night-day", ThemeArg::TokyoNightDay),
    ("everforest-dark", ThemeArg::EverforestDark),
    ("everforest-light", ThemeArg::EverforestLight),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppearanceArg {
    Light,
    Dark,
    System,
}

const APPEARANCE_CHOICES: [(&str, AppearanceArg); 3] = [
    ("light", AppearanceArg::Light),
    ("dark", AppearanceArg::Dark),
    ("system", AppearanceArg::System),
];

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

    pub(crate) fn valid_values_display() -> String {
        Self::choices()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(crate) fn built_in_theme_names_display() -> String {
    ThemeArg::valid_values_display()
}

impl AppearanceArg {
    fn choices() -> &'static [(&'static str, AppearanceArg)] {
        &APPEARANCE_CHOICES
    }

    pub fn from_str(s: &str) -> Option<Self> {
        let normalized = s.trim().to_ascii_lowercase();
        Self::choices().iter().find_map(|(name, appearance)| {
            if *name == normalized {
                Some(*appearance)
            } else {
                None
            }
        })
    }

    pub(crate) fn valid_values_display() -> String {
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
        ThemeArg::AyuLight => Theme::ayu_light(),
        ThemeArg::AyuMirage => Theme::ayu_mirage(),
        ThemeArg::Onedark => Theme::onedark(),
        ThemeArg::GithubLight => Theme::github_light(),
        ThemeArg::GithubDark => Theme::github_dark(),
        ThemeArg::CatppuccinLatte => Theme::catppuccin_latte(),
        ThemeArg::CatppuccinFrappe => Theme::catppuccin_frappe(),
        ThemeArg::CatppuccinMacchiato => Theme::catppuccin_macchiato(),
        ThemeArg::CatppuccinMocha => Theme::catppuccin_mocha(),
        ThemeArg::GruvboxDark => Theme::gruvbox_dark(),
        ThemeArg::GruvboxLight => Theme::gruvbox_light(),
        ThemeArg::NordDark => Theme::nord_dark(),
        ThemeArg::NordLight => Theme::nord_light(),
        ThemeArg::NordDarkHighContrast => Theme::nord_dark_high_contrast(),
        ThemeArg::NordLightHighContrast => Theme::nord_light_high_contrast(),
        ThemeArg::SolarizedLight => Theme::solarized_light(),
        ThemeArg::SolarizedDark => Theme::solarized_dark(),
        ThemeArg::TokyoNightStorm => Theme::tokyo_night_storm(),
        ThemeArg::TokyoNightDay => Theme::tokyo_night_day(),
        ThemeArg::EverforestDark => Theme::everforest_dark(),
        ThemeArg::EverforestLight => Theme::everforest_light(),
    }
}

fn resolve_appearance(appearance: AppearanceArg) -> ThemeArg {
    match appearance {
        AppearanceArg::Light => ThemeArg::Light,
        AppearanceArg::Dark => ThemeArg::Dark,
        AppearanceArg::System => {
            if is_system_dark_mode().unwrap_or(true) {
                ThemeArg::Dark
            } else {
                ThemeArg::Light
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn is_system_dark_mode() -> Option<bool> {
    let output = Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .ok()?;

    if !output.status.success() {
        return Some(false);
    }

    let value = String::from_utf8_lossy(&output.stdout);
    Some(value.trim().eq_ignore_ascii_case("dark"))
}

#[cfg(target_os = "windows")]
fn is_system_dark_mode() -> Option<bool> {
    let output = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "AppsUseLightTheme",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout);
    if value.contains("0x0") {
        Some(true)
    } else if value.contains("0x1") {
        Some(false)
    } else {
        None
    }
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn is_system_dark_mode() -> Option<bool> {
    let color_scheme = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok();
    if let Some(output) = color_scheme
        && output.status.success()
    {
        let value = String::from_utf8_lossy(&output.stdout);
        if value.contains("prefer-dark") {
            return Some(true);
        }
        if value.contains("default") || value.contains("prefer-light") {
            return Some(false);
        }
    }

    let gtk_theme = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
        .output()
        .ok();
    if let Some(output) = gtk_theme
        && output.status.success()
    {
        let value = String::from_utf8_lossy(&output.stdout);
        if value.to_ascii_lowercase().contains("dark") {
            return Some(true);
        }
        return Some(false);
    }

    None
}

pub fn resolve_appearance_arg_with_config(
    cli_appearance: Option<AppearanceArg>,
    config_appearance: Option<&str>,
) -> (AppearanceArg, Vec<String>) {
    let mut warnings = Vec::new();

    if let Some(appearance) = cli_appearance {
        return (appearance, warnings);
    }

    if let Some(config_appearance) = config_appearance {
        if let Some(appearance) = AppearanceArg::from_str(config_appearance) {
            return (appearance, warnings);
        }

        let valid_values = AppearanceArg::valid_values_display();
        warnings.push(format!(
            "Warning: Unknown appearance '{config_appearance}' in config, using system. Valid options: {valid_values}"
        ));
    }

    (AppearanceArg::System, warnings)
}

fn is_dark_color(c: Color) -> bool {
    match c {
        Color::Rgb(r, g, b) => (u16::from(r) + u16::from(g) + u16::from(b)) / 3 < 128,
        _ => true,
    }
}

fn fallback_embedded_theme_for_panel_bg(panel_bg: Color) -> EmbeddedThemeName {
    if is_dark_color(panel_bg) {
        EmbeddedThemeName::Base16EightiesDark
    } else {
        EmbeddedThemeName::Base16OceanLight
    }
}

fn is_supported_color_value(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    if let Some(hex) = normalized.strip_prefix('#') {
        return hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit());
    }

    matches!(
        normalized.as_str(),
        "black"
            | "red"
            | "green"
            | "yellow"
            | "blue"
            | "magenta"
            | "cyan"
            | "gray"
            | "grey"
            | "darkgray"
            | "dark_gray"
            | "darkgrey"
            | "dark_grey"
            | "lightred"
            | "light_red"
            | "lightgreen"
            | "light_green"
            | "lightyellow"
            | "light_yellow"
            | "lightblue"
            | "light_blue"
            | "lightmagenta"
            | "light_magenta"
            | "lightcyan"
            | "light_cyan"
            | "white"
    )
}

fn parse_color_value(value: &str) -> Option<Color> {
    let normalized = value.trim().to_ascii_lowercase();
    if let Some(hex) = normalized.strip_prefix('#') {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }

    Some(match normalized.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "dark_gray" | "darkgrey" | "dark_grey" => Color::DarkGray,
        "lightred" | "light_red" => Color::LightRed,
        "lightgreen" | "light_green" => Color::LightGreen,
        "lightyellow" | "light_yellow" => Color::LightYellow,
        "lightblue" | "light_blue" => Color::LightBlue,
        "lightmagenta" | "light_magenta" => Color::LightMagenta,
        "lightcyan" | "light_cyan" => Color::LightCyan,
        "white" => Color::White,
        _ => return None,
    })
}

fn require_local_theme_color(table: &toml::Table, key: &str) -> Result<Color, String> {
    let value = table
        .get(key)
        .ok_or_else(|| format!("Theme key '{key}' is required"))?;
    let raw = value
        .as_str()
        .ok_or_else(|| format!("Theme key '{key}' must be a string"))?;
    if !is_supported_color_value(raw) {
        return Err(format!(
            "Theme key '{key}' must be a named color or #RRGGBB"
        ));
    }
    parse_color_value(raw).ok_or_else(|| format!("Theme key '{key}' could not be parsed"))
}

fn parse_optional_local_theme_string(
    table: &toml::Table,
    key: &str,
) -> Result<Option<String>, String> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let raw = value
        .as_str()
        .ok_or_else(|| format!("Theme key '{key}' must be a string"))?
        .trim()
        .to_string();
    if raw.is_empty() {
        return Err(format!("Theme key '{key}' cannot be empty"));
    }
    Ok(Some(raw))
}

fn load_custom_syntect_theme(
    theme_path: &Path,
    syntax_theme: &str,
) -> Result<syntect::highlighting::Theme, String> {
    let syntax_path = Path::new(syntax_theme);
    let resolved = if syntax_path.is_absolute() {
        syntax_path.to_path_buf()
    } else {
        theme_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(syntax_path)
    };

    if resolved.extension().and_then(|ext| ext.to_str()) != Some("tmTheme") {
        return Err(format!(
            "Theme key 'syntax_theme' must point to a .tmTheme file; got '{}'",
            resolved.display()
        ));
    }

    ThemeSet::get_theme(&resolved).map_err(|err| {
        format!(
            "Failed to load syntax theme '{}': {err}",
            resolved.display()
        )
    })
}

// Keep the local theme schema explicit so unknown-key warnings, docs, and
// tests stay aligned even though Rust cannot derive TOML field names from the
// `Theme` struct itself.
const LOCAL_THEME_KEYS: &[&str] = &[
    "panel_bg",
    "bg_highlight",
    "fg_primary",
    "fg_secondary",
    "fg_dim",
    "diff_add",
    "diff_add_bg",
    "diff_del",
    "diff_del_bg",
    "diff_context",
    "diff_hunk_header",
    "expanded_context_fg",
    "syntax_add_bg",
    "syntax_del_bg",
    "syntax_theme",
    "file_added",
    "file_modified",
    "file_deleted",
    "file_renamed",
    "reviewed",
    "pending",
    "comment_note",
    "comment_suggestion",
    "comment_issue",
    "comment_praise",
    "border_focused",
    "border_unfocused",
    "status_bar_bg",
    "cursor_color",
    "cursor_line_bg",
    "branch_name",
    "help_indicator",
    "message_info_fg",
    "message_info_bg",
    "message_warning_fg",
    "message_warning_bg",
    "message_error_fg",
    "message_error_bg",
    "update_badge_fg",
    "update_badge_bg",
    "mode_fg",
    "mode_bg",
];

fn load_local_theme_from_path(path: &Path) -> Result<(Theme, Vec<String>), String> {
    let contents = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read local theme '{}': {err}", path.display()))?;
    let value: toml::Value = toml::from_str(&contents)
        .map_err(|err| format!("Failed to parse local theme '{}': {err}", path.display()))?;
    let table = value
        .as_table()
        .ok_or_else(|| format!("Local theme '{}' must be a TOML table", path.display()))?;

    let mut warnings = Vec::new();
    for key in table.keys() {
        if !LOCAL_THEME_KEYS.contains(&key.as_str()) {
            warnings.push(format!(
                "Warning: Unknown local theme key '{key}' in '{}', ignoring",
                path.display()
            ));
        }
    }

    let panel_bg = require_local_theme_color(table, "panel_bg")?;
    let syntax_theme = parse_optional_local_theme_string(table, "syntax_theme")?;
    let syntax_theme = match syntax_theme.as_deref() {
        Some(value) => SyntaxThemeSource::Custom(Box::new(load_custom_syntect_theme(path, value)?)),
        None => SyntaxThemeSource::Embedded(fallback_embedded_theme_for_panel_bg(panel_bg)),
    };

    let theme = Theme {
        highlighter: OnceLock::new(),
        panel_bg,
        bg_highlight: require_local_theme_color(table, "bg_highlight")?,
        fg_primary: require_local_theme_color(table, "fg_primary")?,
        fg_secondary: require_local_theme_color(table, "fg_secondary")?,
        fg_dim: require_local_theme_color(table, "fg_dim")?,
        diff_add: require_local_theme_color(table, "diff_add")?,
        diff_add_bg: require_local_theme_color(table, "diff_add_bg")?,
        diff_del: require_local_theme_color(table, "diff_del")?,
        diff_del_bg: require_local_theme_color(table, "diff_del_bg")?,
        diff_context: require_local_theme_color(table, "diff_context")?,
        diff_hunk_header: require_local_theme_color(table, "diff_hunk_header")?,
        expanded_context_fg: require_local_theme_color(table, "expanded_context_fg")?,
        syntax_add_bg: require_local_theme_color(table, "syntax_add_bg")?,
        syntax_del_bg: require_local_theme_color(table, "syntax_del_bg")?,
        syntax_theme,
        file_added: require_local_theme_color(table, "file_added")?,
        file_modified: require_local_theme_color(table, "file_modified")?,
        file_deleted: require_local_theme_color(table, "file_deleted")?,
        file_renamed: require_local_theme_color(table, "file_renamed")?,
        reviewed: require_local_theme_color(table, "reviewed")?,
        pending: require_local_theme_color(table, "pending")?,
        comment_note: require_local_theme_color(table, "comment_note")?,
        comment_suggestion: require_local_theme_color(table, "comment_suggestion")?,
        comment_issue: require_local_theme_color(table, "comment_issue")?,
        comment_praise: require_local_theme_color(table, "comment_praise")?,
        border_focused: require_local_theme_color(table, "border_focused")?,
        border_unfocused: require_local_theme_color(table, "border_unfocused")?,
        status_bar_bg: require_local_theme_color(table, "status_bar_bg")?,
        cursor_color: require_local_theme_color(table, "cursor_color")?,
        cursor_line_bg: require_local_theme_color(table, "cursor_line_bg")?,
        branch_name: require_local_theme_color(table, "branch_name")?,
        help_indicator: require_local_theme_color(table, "help_indicator")?,
        message_info_fg: require_local_theme_color(table, "message_info_fg")?,
        message_info_bg: require_local_theme_color(table, "message_info_bg")?,
        message_warning_fg: require_local_theme_color(table, "message_warning_fg")?,
        message_warning_bg: require_local_theme_color(table, "message_warning_bg")?,
        message_error_fg: require_local_theme_color(table, "message_error_fg")?,
        message_error_bg: require_local_theme_color(table, "message_error_bg")?,
        update_badge_fg: require_local_theme_color(table, "update_badge_fg")?,
        update_badge_bg: require_local_theme_color(table, "update_badge_bg")?,
        mode_fg: require_local_theme_color(table, "mode_fg")?,
        mode_bg: require_local_theme_color(table, "mode_bg")?,
    };

    Ok((theme, warnings))
}

fn normalize_local_theme_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("theme name cannot be empty".to_string());
    }

    let path = Path::new(trimmed);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("theme name must be a plain name, not a path".to_string());
    }

    Ok(trimmed.to_ascii_lowercase())
}

fn resolve_theme_name(
    name: &str,
    theme_dir: &Path,
) -> Result<Option<(Theme, Vec<String>)>, String> {
    if let Some(theme) = ThemeArg::from_str(name) {
        return Ok(Some((resolve_theme(theme), Vec::new())));
    }

    let normalized = normalize_local_theme_name(name)?;
    let path = theme_dir.join(format!("{normalized}.toml"));
    if !path.exists() {
        return Ok(None);
    }

    load_local_theme_from_path(&path).map(Some)
}

fn resolve_config_theme_name(key: &str, value: Option<&str>) -> (Option<Theme>, Vec<String>) {
    let mut warnings = Vec::new();
    let Some(value) = value else {
        return (None, warnings);
    };

    let theme_dir = match themes_dir() {
        Ok(path) => path,
        Err(err) => {
            warnings.push(format!(
                "Warning: Could not determine theme directory for config key '{key}': {err}"
            ));
            return (None, warnings);
        }
    };

    match resolve_theme_name(value, &theme_dir) {
        Ok(Some((theme, mut theme_warnings))) => {
            warnings.append(&mut theme_warnings);
            (Some(theme), warnings)
        }
        Ok(None) => {
            let valid_values = built_in_theme_names_display();
            warnings.push(format!(
                "Warning: Unknown theme '{value}' in config key '{key}', ignoring. Bundled themes: {valid_values}"
            ));
            (None, warnings)
        }
        Err(err) => {
            warnings.push(format!(
                "Warning: Failed to load theme '{value}' from config key '{key}': {err}"
            ));
            (None, warnings)
        }
    }
}

pub fn resolve_theme_with_config(
    cli_theme: Option<String>,
    cli_appearance: Option<AppearanceArg>,
    config_theme: Option<&str>,
    config_theme_dark: Option<&str>,
    config_theme_light: Option<&str>,
    config_appearance: Option<&str>,
) -> Result<(Theme, Vec<String>), String> {
    let mut warnings = Vec::new();
    let (appearance_arg, appearance_warnings) =
        resolve_appearance_arg_with_config(cli_appearance, config_appearance);
    warnings.extend(appearance_warnings);
    let (theme_dark, dark_warnings) = resolve_config_theme_name("theme_dark", config_theme_dark);
    warnings.extend(dark_warnings);
    let (theme_light, light_warnings) =
        resolve_config_theme_name("theme_light", config_theme_light);
    warnings.extend(light_warnings);

    if let Some(cli_theme) = cli_theme {
        let theme_dir =
            themes_dir().map_err(|err| format!("Could not determine theme directory: {err}"))?;
        let (theme, mut theme_warnings) = resolve_theme_name(&cli_theme, &theme_dir)?
            .ok_or_else(|| {
                let valid_values = built_in_theme_names_display();
                format!(
                    "Unknown theme '{cli_theme}'. Bundled themes: {valid_values}. Local themes are loaded from {}",
                    theme_dir.display()
                )
            })?;
        warnings.append(&mut theme_warnings);
        if cli_appearance.is_some() || config_appearance.is_some() {
            warnings.push(
                "Warning: Appearance setting is ignored when theme is explicitly set".to_string(),
            );
        }
        return Ok((theme, warnings));
    }

    if let Some(config_theme) = config_theme {
        match themes_dir() {
            Ok(theme_dir) => match resolve_theme_name(config_theme, &theme_dir) {
                Ok(Some((theme, mut theme_warnings))) => {
                    warnings.append(&mut theme_warnings);
                    if cli_appearance.is_some() || config_appearance.is_some() {
                        warnings.push(
                            "Warning: Appearance setting is ignored when theme is explicitly set"
                                .to_string(),
                        );
                    }
                    return Ok((theme, warnings));
                }
                Ok(None) => {
                    let valid_values = built_in_theme_names_display();
                    warnings.push(format!(
                        "Warning: Unknown theme '{config_theme}' in config, using appearance mode. Bundled themes: {valid_values}"
                    ));
                }
                Err(err) => warnings.push(format!(
                    "Warning: Failed to load theme '{config_theme}' from config: {err}"
                )),
            },
            Err(err) => warnings.push(format!(
                "Warning: Could not determine theme directory while resolving config theme '{config_theme}': {err}"
            )),
        }
    }

    let resolved = match (theme_dark, theme_light) {
        (Some(theme_dark), Some(theme_light)) => match appearance_arg {
            AppearanceArg::Dark => theme_dark,
            AppearanceArg::Light => theme_light,
            AppearanceArg::System => {
                if is_system_dark_mode().unwrap_or(true) {
                    theme_dark
                } else {
                    theme_light
                }
            }
        },
        (Some(theme_dark), None) => {
            if cli_appearance.is_some() || config_appearance.is_some() {
                warnings.push(
                    "Warning: Appearance setting is ignored when only theme_dark is configured"
                        .to_string(),
                );
            }
            theme_dark
        }
        (None, Some(theme_light)) => {
            if cli_appearance.is_some() || config_appearance.is_some() {
                warnings.push(
                    "Warning: Appearance setting is ignored when only theme_light is configured"
                        .to_string(),
                );
            }
            theme_light
        }
        (None, None) => resolve_theme(resolve_appearance(appearance_arg)),
    };

    Ok((resolved, warnings))
}

impl Theme {
    /// Get the syntax highlighter for this theme (lazily initialized, cached)
    pub fn syntax_highlighter(&self) -> &SyntaxHighlighter {
        self.highlighter.get_or_init(|| match &self.syntax_theme {
            SyntaxThemeSource::Embedded(theme) => {
                SyntaxHighlighter::new(*theme, self.syntax_add_bg, self.syntax_del_bg)
            }
            SyntaxThemeSource::Custom(theme) => SyntaxHighlighter::with_theme(
                *theme.clone(),
                self.syntax_add_bg,
                self.syntax_del_bg,
            ),
        })
    }

    #[cfg(test)]
    pub fn embedded_syntax_theme_name(&self) -> Option<EmbeddedThemeName> {
        match self.syntax_theme {
            SyntaxThemeSource::Embedded(theme) => Some(theme),
            SyntaxThemeSource::Custom(_) => None,
        }
    }

    #[cfg(test)]
    pub fn uses_custom_syntax_theme(&self) -> bool {
        matches!(self.syntax_theme, SyntaxThemeSource::Custom(_))
    }

    /// Subtle row-tint for diff section markers (hunk headers, gap
    /// expanders, hidden-line stubs) — derived as a brightness shift from
    /// `panel_bg` so it adapts across all themed palettes without per-theme
    /// tuning.
    pub fn section_highlight_bg(&self) -> Color {
        shift_lightness(self.panel_bg, 18)
    }
}

fn shift_lightness(c: Color, amount: i32) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            let avg = (r as i32 + g as i32 + b as i32) / 3;
            let amt = if avg < 128 { amount } else { -amount };
            Color::Rgb(
                (r as i32 + amt).clamp(0, 255) as u8,
                (g as i32 + amt).clamp(0, 255) as u8,
                (b as i32 + amt).clamp(0, 255) as u8,
            )
        }
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        fs,
        path::{Path, PathBuf},
    };
    use tempfile::tempdir;

    fn sample_tm_theme() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>name</key>
  <string>Fixture Theme</string>
  <key>settings</key>
  <array>
    <dict>
      <key>settings</key>
      <dict>
        <key>foreground</key>
        <string>#c3ccdc</string>
        <key>background</key>
        <string>#011627</string>
      </dict>
    </dict>
  </array>
</dict>
</plist>
"#
    }

    fn sample_local_theme_body(extra: &str) -> String {
        format!(
            r##"panel_bg = "#011627"
bg_highlight = "#1d3b53"
fg_primary = "#c3ccdc"
fg_secondary = "#a1aab8"
fg_dim = "#7c8f9e"
diff_add = "#a1cd5e"
diff_add_bg = "#13311f"
diff_del = "#ef5350"
diff_del_bg = "#341a1a"
diff_context = "#c3ccdc"
diff_hunk_header = "#82aaff"
expanded_context_fg = "#7c8f9e"
syntax_add_bg = "#10281a"
syntax_del_bg = "#2d1418"
file_added = "#a1cd5e"
file_modified = "#e3d18a"
file_deleted = "#ef5350"
file_renamed = "#c792ea"
reviewed = "#a1cd5e"
pending = "#e3d18a"
comment_note = "#7fdbca"
comment_suggestion = "#82aaff"
comment_issue = "#ef5350"
comment_praise = "#a1cd5e"
border_focused = "#82aaff"
border_unfocused = "#4b6479"
status_bar_bg = "#0b253a"
cursor_color = "#ffcb8b"
cursor_line_bg = "#0f2a3f"
branch_name = "#7fdbca"
help_indicator = "#4b6479"
message_info_fg = "black"
message_info_bg = "#7fdbca"
message_warning_fg = "black"
message_warning_bg = "#ffcb8b"
message_error_fg = "white"
message_error_bg = "#ef5350"
update_badge_fg = "black"
update_badge_bg = "#ffcb8b"
mode_fg = "#011627"
mode_bg = "#82aaff"
{extra}
"##
        )
    }

    fn write_local_theme(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(format!("{name}.toml"));
        fs::write(&path, body).expect("failed to write local theme");
        path
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
        let (resolved, warnings) = resolve_theme_with_config(
            Some("light".to_string()),
            None,
            Some("dark"),
            None,
            None,
            None,
        )
        .expect("theme resolution should succeed");
        assert_eq!(
            resolved.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::Base16OceanLight)
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_use_config_theme_when_cli_missing() {
        let (resolved, warnings) =
            resolve_theme_with_config(None, None, Some("light"), None, None, None)
                .expect("theme resolution should succeed");
        assert_eq!(
            resolved.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::Base16OceanLight)
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_fallback_to_appearance_and_warn_for_invalid_config_theme() {
        let (resolved, warnings) = resolve_theme_with_config(
            None,
            Some(AppearanceArg::Dark),
            Some("unknown"),
            None,
            None,
            None,
        )
        .expect("theme resolution should succeed");
        assert_eq!(
            resolved.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::Base16EightiesDark)
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Unknown theme 'unknown'"));
    }

    #[test]
    fn should_fallback_to_appearance_when_no_theme_is_set() {
        let (resolved, warnings) =
            resolve_theme_with_config(None, Some(AppearanceArg::Dark), None, None, None, None)
                .expect("theme resolution should succeed");
        assert_eq!(
            resolved.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::Base16EightiesDark)
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_use_catppuccin_theme_from_config_when_cli_missing() {
        let (resolved, warnings) =
            resolve_theme_with_config(None, None, Some("catppuccin-fRappe"), None, None, None)
                .expect("theme resolution should succeed");
        assert_eq!(
            resolved.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::CatppuccinFrappe)
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_resolve_local_theme_name_from_directory() {
        let dir = tempdir().expect("failed to create temp dir");
        let tm_theme_path = dir.path().join("example.tmTheme");
        fs::write(&tm_theme_path, sample_tm_theme()).expect("failed to write tmTheme");
        write_local_theme(
            dir.path(),
            "local-teal",
            &sample_local_theme_body(r#"syntax_theme = "example.tmTheme""#),
        );

        let (theme, warnings) = resolve_theme_name("local-teal", dir.path())
            .expect("theme resolution should succeed")
            .expect("theme should exist");
        assert_eq!(theme.panel_bg, Color::Rgb(1, 22, 39));
        assert!(theme.uses_custom_syntax_theme());
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_warn_on_unknown_local_theme_key() {
        let dir = tempdir().expect("failed to create temp dir");
        fs::write(dir.path().join("example.tmTheme"), sample_tm_theme())
            .expect("failed to write tmTheme");
        let body = format!(
            "{}\nextra_key = \"ignored\"\n",
            sample_local_theme_body(r#"syntax_theme = "example.tmTheme""#)
        );
        let path = write_local_theme(dir.path(), "local-teal", &body);

        let (_, warnings) =
            load_local_theme_from_path(&path).expect("local theme should load successfully");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Unknown local theme key 'extra_key'"));
    }

    #[test]
    fn should_reject_invalid_local_theme_color() {
        let dir = tempdir().expect("failed to create temp dir");
        let body = sample_local_theme_body("");
        let path = write_local_theme(
            dir.path(),
            "local-teal",
            &body.replace(r##"fg_primary = "#c3ccdc""##, r#"fg_primary = "oops""#),
        );

        let err = match load_local_theme_from_path(&path) {
            Ok(_) => panic!("theme should fail"),
            Err(err) => err,
        };
        assert!(err.contains("Theme key 'fg_primary'"));
    }

    #[test]
    fn should_load_checked_in_tuicr_teal_example() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("tuicr-teal.toml");

        let (theme, warnings) =
            load_local_theme_from_path(&path).expect("checked-in example should load");
        assert!(warnings.is_empty());
        assert_eq!(theme.panel_bg, Color::Rgb(6, 40, 50));
        assert_eq!(theme.mode_bg, Color::Rgb(78, 227, 255));
        assert!(theme.uses_custom_syntax_theme());
    }

    #[test]
    fn should_default_to_system_appearance_when_not_set() {
        let (resolved, warnings) = resolve_appearance_arg_with_config(None, None);
        assert_eq!(resolved, AppearanceArg::System);
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_use_appearance_when_theme_is_not_set() {
        let (resolved, warnings) =
            resolve_theme_with_config(None, Some(AppearanceArg::Light), None, None, None, None)
                .expect("theme resolution should succeed");
        assert_eq!(
            resolved.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::Base16OceanLight)
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_select_variant_theme_by_appearance_when_both_variants_configured() {
        let (resolved, warnings) = resolve_theme_with_config(
            None,
            Some(AppearanceArg::Light),
            None,
            Some("gruvbox-dark"),
            Some("gruvbox-light"),
            None,
        )
        .expect("theme resolution should succeed");
        assert_eq!(
            resolved.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::GruvboxLight)
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_ignore_appearance_when_only_dark_variant_configured() {
        let (resolved, warnings) = resolve_theme_with_config(
            None,
            Some(AppearanceArg::Light),
            None,
            Some("gruvbox-dark"),
            None,
            None,
        )
        .expect("theme resolution should succeed");
        assert_eq!(
            resolved.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::GruvboxDark)
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("only theme_dark is configured"));
    }

    #[test]
    fn should_resolve_catppuccin_mocha_syntect_theme() {
        let theme = resolve_theme(ThemeArg::CatppuccinMocha);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::CatppuccinMocha)
        );
    }

    #[test]
    fn should_resolve_catppuccin_latte_syntect_theme() {
        let theme = resolve_theme(ThemeArg::CatppuccinLatte);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::CatppuccinLatte)
        );
    }

    #[test]
    fn should_resolve_nord_dark_to_nord_syntect_theme() {
        let theme = resolve_theme(ThemeArg::NordDark);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::Nord)
        );
    }

    #[test]
    fn should_resolve_nord_light_to_ocean_light_syntect_theme() {
        let theme = resolve_theme(ThemeArg::NordLight);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::Base16OceanLight)
        );
    }

    #[test]
    fn should_resolve_nord_dark_high_contrast_to_nord_syntect_theme() {
        let theme = resolve_theme(ThemeArg::NordDarkHighContrast);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::Nord)
        );
    }

    #[test]
    fn should_resolve_nord_light_high_contrast_to_ocean_light_syntect_theme() {
        let theme = resolve_theme(ThemeArg::NordLightHighContrast);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::Base16OceanLight)
        );
    }

    #[test]
    fn should_use_dark_bg_for_nord_dark_mode_foreground() {
        let theme = Theme::nord_dark();
        assert_eq!(theme.mode_fg, Color::Rgb(46, 52, 64)); // nord0
    }

    #[test]
    fn should_use_fg1_for_nord_light_mode_foreground() {
        let theme = Theme::nord_light();
        assert_eq!(theme.mode_fg, Color::Rgb(59, 66, 82)); // nord1
    }

    #[test]
    fn should_boost_fg_primary_for_nord_dark_high_contrast() {
        let dark = resolve_theme(ThemeArg::NordDark);
        let hc = resolve_theme(ThemeArg::NordDarkHighContrast);
        assert_ne!(dark.fg_primary, hc.fg_primary);
        assert_eq!(hc.fg_primary, Color::Rgb(236, 239, 244)); // nord6
    }

    #[test]
    fn should_deepen_fg_dim_for_nord_light_high_contrast() {
        let light = resolve_theme(ThemeArg::NordLight);
        let hc = resolve_theme(ThemeArg::NordLightHighContrast);
        assert_ne!(light.fg_dim, hc.fg_dim);
        assert_eq!(hc.fg_dim, Color::Rgb(67, 76, 94)); // nord2
    }

    #[test]
    fn should_resolve_everforest_dark_to_gruvbox_dark_syntect_theme() {
        let theme = resolve_theme(ThemeArg::EverforestDark);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::GruvboxDark)
        );
    }

    #[test]
    fn should_resolve_everforest_light_to_gruvbox_light_syntect_theme() {
        let theme = resolve_theme(ThemeArg::EverforestLight);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::GruvboxLight)
        );
    }

    #[test]
    fn should_use_medium_variant_bg0_for_everforest_dark() {
        // Pin the medium-variant bg0 so a hard/medium swap can't slip in.
        let theme = Theme::everforest_dark();
        assert_eq!(theme.panel_bg, Color::Rgb(45, 53, 59)); // #2d353b
    }

    #[test]
    fn should_use_dark_fg_on_bright_accents_for_everforest_light() {
        // Light variant must use the dark fg (#5c6a72) on bright accent
        // backgrounds — using bg0 would produce near-white on near-white.
        let theme = Theme::everforest_light();
        assert_eq!(theme.mode_fg, Color::Rgb(92, 106, 114)); // #5c6a72
    }

    #[test]
    fn should_resolve_gruvbox_dark_to_dark_syntect_theme() {
        let theme = resolve_theme(ThemeArg::GruvboxDark);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::GruvboxDark)
        );
    }

    #[test]
    fn should_resolve_gruvbox_light_to_light_syntect_theme() {
        let theme = resolve_theme(ThemeArg::GruvboxLight);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::GruvboxLight)
        );
    }

    #[test]
    fn should_resolve_ayu_light_to_onehalf_light_syntect_theme() {
        let theme = resolve_theme(ThemeArg::AyuLight);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::OneHalfLight)
        );
    }

    #[test]
    fn should_resolve_onedark_to_onehalf_dark_syntect_theme() {
        let theme = resolve_theme(ThemeArg::Onedark);
        assert_eq!(
            theme.embedded_syntax_theme_name(),
            Some(EmbeddedThemeName::OneHalfDark)
        );
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
