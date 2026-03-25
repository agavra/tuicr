//! Theme support for tuicr
//!
//! Provides dark and light themes with automatic terminal background detection.

use std::{process::Command, sync::OnceLock};

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

            syntect_theme: EmbeddedThemeName::SolarizedLight,

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

            syntect_theme: EmbeddedThemeName::SolarizedDark,

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
            syntect_theme: EmbeddedThemeName::OneHalfLight,

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
            syntect_theme: EmbeddedThemeName::OneHalfDark,

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
        syntect_theme,

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

        syntect_theme: flavor.syntect_theme,

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
    Onedark,
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
}

const THEME_CHOICES: [(&str, ThemeArg); 16] = [
    ("dark", ThemeArg::Dark),
    ("light", ThemeArg::Light),
    ("ayu-light", ThemeArg::AyuLight),
    ("onedark", ThemeArg::Onedark),
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
];

/// CLI arguments parsed from command line
#[derive(Debug, Clone, Default)]
pub struct CliArgs {
    pub theme: Option<ThemeArg>,
    pub appearance: Option<AppearanceArg>,
    /// Output to stdout instead of clipboard when exporting
    pub output_to_stdout: bool,
    /// Skip checking for updates on startup
    pub no_update_check: bool,
    /// Commit/revision range to review
    pub revisions: Option<String>,
    /// Skip commit selector and review uncommitted changes directly
    pub working_tree: bool,
}

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

    fn valid_values_display() -> String {
        Self::choices()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }
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
        ThemeArg::AyuLight => Theme::ayu_light(),
        ThemeArg::Onedark => Theme::onedark(),
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

pub fn resolve_theme_arg_with_config(
    cli_theme: Option<ThemeArg>,
    config_theme: Option<&str>,
) -> (Option<ThemeArg>, Vec<String>) {
    let mut warnings = Vec::new();

    if let Some(theme) = cli_theme {
        return (Some(theme), warnings);
    }

    if let Some(config_theme) = config_theme {
        if let Some(theme) = ThemeArg::from_str(config_theme) {
            return (Some(theme), warnings);
        }

        let valid_values = ThemeArg::valid_values_display();
        warnings.push(format!(
            "Warning: Unknown theme '{config_theme}' in config, using appearance mode. Valid options: {valid_values}"
        ));
    }

    (None, warnings)
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

fn parse_theme_variant_from_config(
    key: &str,
    value: Option<&str>,
) -> (Option<ThemeArg>, Vec<String>) {
    let mut warnings = Vec::new();
    let Some(value) = value else {
        return (None, warnings);
    };

    if let Some(theme) = ThemeArg::from_str(value) {
        return (Some(theme), warnings);
    }

    let valid_values = ThemeArg::valid_values_display();
    warnings.push(format!(
        "Warning: Unknown theme '{value}' in config key '{key}', ignoring. Valid options: {valid_values}"
    ));
    (None, warnings)
}

pub fn resolve_theme_with_config(
    cli_theme: Option<ThemeArg>,
    cli_appearance: Option<AppearanceArg>,
    config_theme: Option<&str>,
    config_theme_dark: Option<&str>,
    config_theme_light: Option<&str>,
    config_appearance: Option<&str>,
) -> (Theme, Vec<String>) {
    let (theme_arg, mut warnings) = resolve_theme_arg_with_config(cli_theme, config_theme);
    let (appearance_arg, appearance_warnings) =
        resolve_appearance_arg_with_config(cli_appearance, config_appearance);
    warnings.extend(appearance_warnings);
    let (theme_dark_arg, dark_warnings) =
        parse_theme_variant_from_config("theme_dark", config_theme_dark);
    warnings.extend(dark_warnings);
    let (theme_light_arg, light_warnings) =
        parse_theme_variant_from_config("theme_light", config_theme_light);
    warnings.extend(light_warnings);

    if let Some(theme_arg) = theme_arg {
        if cli_appearance.is_some() || config_appearance.is_some() {
            warnings.push(
                "Warning: Appearance setting is ignored when theme is explicitly set".to_string(),
            );
        }
        (resolve_theme(theme_arg), warnings)
    } else {
        match (theme_dark_arg, theme_light_arg) {
            (Some(theme_dark), Some(theme_light)) => {
                let resolved = match appearance_arg {
                    AppearanceArg::Dark => theme_dark,
                    AppearanceArg::Light => theme_light,
                    AppearanceArg::System => {
                        if is_system_dark_mode().unwrap_or(true) {
                            theme_dark
                        } else {
                            theme_light
                        }
                    }
                };
                (resolve_theme(resolved), warnings)
            }
            (Some(theme_dark), None) => {
                if cli_appearance.is_some() || config_appearance.is_some() {
                    warnings.push(
                        "Warning: Appearance setting is ignored when only theme_dark is configured"
                            .to_string(),
                    );
                }
                (resolve_theme(theme_dark), warnings)
            }
            (None, Some(theme_light)) => {
                if cli_appearance.is_some() || config_appearance.is_some() {
                    warnings.push(
                        "Warning: Appearance setting is ignored when only theme_light is configured"
                            .to_string(),
                    );
                }
                (resolve_theme(theme_light), warnings)
            }
            (None, None) => (resolve_theme(resolve_appearance(appearance_arg)), warnings),
        }
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

/// Print version and exit
fn print_version() -> ! {
    println!("tuicr {}", env!("CARGO_PKG_VERSION"));
    std::process::exit(0);
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
    let appearance_values = AppearanceArg::valid_values_display();
    let config_path = config_path_hint();
    println!(
        "tuicr - Review AI-generated diffs like a GitHub pull request

Usage: {name} [OPTIONS]

Options:
  -r, --revisions <REVSET>  Commit range/Revset to review (syntax depends on VCS backend)
  --theme <THEME>        Color theme to use
                          Valid values: {valid_values}
  --appearance <MODE>    Appearance mode for default theme
                         Valid values: {appearance_values}
                         Used when no explicit theme is set
                         Precedence: --appearance > {config_path} > system
  -w, --working-tree     Include uncommitted changes (skip commit selector when used alone,
                         combine with commits when used with -r)
  --stdout               Output to stdout instead of clipboard when exporting
  --no-update-check      Skip checking for updates on startup
  -V, --version          Print version
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
        // Handle --version / -V
        if args[i] == "--version" || args[i] == "-V" {
            print_version();
        }

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

        // Handle -w / --working-tree
        if args[i] == "-w" || args[i] == "--working-tree" {
            cli_args.working_tree = true;
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

        // Handle --appearance value
        if args[i] == "--appearance" {
            let valid_values = AppearanceArg::valid_values_display();
            let value = args
                .get(i + 1)
                .ok_or_else(|| format!("--appearance requires a value ({valid_values})"))?;

            if value.starts_with('-') {
                return Err(format!("--appearance requires a value ({valid_values})"));
            }

            cli_args.appearance = AppearanceArg::from_str(value)
                .ok_or_else(|| {
                    format!("Unknown appearance '{value}'. Valid options: {valid_values}")
                })
                .map(Some)?;
        }

        // Handle --appearance=value
        if let Some(value) = args[i].strip_prefix("--appearance=") {
            let valid_values = AppearanceArg::valid_values_display();
            if value.is_empty() {
                return Err(format!("--appearance requires a value ({valid_values})"));
            }

            cli_args.appearance = AppearanceArg::from_str(value)
                .ok_or_else(|| {
                    format!("Unknown appearance '{value}'. Valid options: {valid_values}")
                })
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
    fn should_parse_ayu_light_theme() {
        let parsed =
            parse_for_test(&["tuicr", "--theme", "ayu-light"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some(ThemeArg::AyuLight));
    }

    #[test]
    fn should_parse_onedark_theme() {
        let parsed =
            parse_for_test(&["tuicr", "--theme", "onedark"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some(ThemeArg::Onedark));
    }

    #[test]
    fn should_parse_gruvbox_themes() {
        let parsed =
            parse_for_test(&["tuicr", "--theme", "gruvbox-dark"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some(ThemeArg::GruvboxDark));

        let parsed =
            parse_for_test(&["tuicr", "--theme=gruvbox-light"]).expect("parse should succeed");
        assert_eq!(parsed.theme, Some(ThemeArg::GruvboxLight));
    }

    #[test]
    fn should_leave_theme_none_when_not_provided() {
        let parsed = parse_for_test(&["tuicr"]).expect("parse should succeed");
        assert_eq!(parsed.theme, None);
    }

    #[test]
    fn should_parse_working_tree_short_flag() {
        let parsed = parse_for_test(&["tuicr", "-w"]).expect("parse should succeed");
        assert!(parsed.working_tree);
    }

    #[test]
    fn should_parse_working_tree_long_flag() {
        let parsed = parse_for_test(&["tuicr", "--working-tree"]).expect("parse should succeed");
        assert!(parsed.working_tree);
    }

    #[test]
    fn should_default_working_tree_to_false() {
        let parsed = parse_for_test(&["tuicr"]).expect("parse should succeed");
        assert!(!parsed.working_tree);
    }

    #[test]
    fn should_parse_working_tree_with_revisions() {
        let parsed =
            parse_for_test(&["tuicr", "-w", "-r", "HEAD~3..HEAD"]).expect("parse should succeed");
        assert!(parsed.working_tree);
        assert_eq!(parsed.revisions, Some("HEAD~3..HEAD".to_string()));
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
    fn should_parse_appearance_when_provided() {
        let parsed =
            parse_for_test(&["tuicr", "--appearance", "system"]).expect("parse should succeed");
        assert_eq!(parsed.appearance, Some(AppearanceArg::System));
    }

    #[test]
    fn should_error_for_invalid_appearance() {
        let err =
            parse_for_test(&["tuicr", "--appearance", "nope"]).expect_err("parse should fail");
        assert!(err.contains("Unknown appearance 'nope'"));
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
        assert_eq!(resolved, Some(ThemeArg::Light));
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_use_config_theme_when_cli_missing() {
        let (resolved, warnings) = resolve_theme_arg_with_config(None, Some("light"));
        assert_eq!(resolved, Some(ThemeArg::Light));
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_fallback_to_appearance_and_warn_for_invalid_config_theme() {
        let (resolved, warnings) = resolve_theme_arg_with_config(None, Some("unknown"));
        assert_eq!(resolved, None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Unknown theme 'unknown'"));
    }

    #[test]
    fn should_fallback_to_appearance_when_no_theme_is_set() {
        let (resolved, warnings) = resolve_theme_arg_with_config(None, None);
        assert_eq!(resolved, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn should_use_catppuccin_theme_from_config_when_cli_missing() {
        let (resolved, warnings) = resolve_theme_arg_with_config(None, Some("catppuccin-fRappe"));
        assert_eq!(resolved, Some(ThemeArg::CatppuccinFrappe));
        assert!(warnings.is_empty());
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
            resolve_theme_with_config(None, Some(AppearanceArg::Light), None, None, None, None);
        assert_eq!(resolved.syntect_theme, EmbeddedThemeName::Base16OceanLight);
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
        );
        assert_eq!(resolved.syntect_theme, EmbeddedThemeName::GruvboxLight);
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
        );
        assert_eq!(resolved.syntect_theme, EmbeddedThemeName::GruvboxDark);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("only theme_dark is configured"));
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
    fn should_resolve_nord_dark_to_nord_syntect_theme() {
        let theme = resolve_theme(ThemeArg::NordDark);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::Nord);
    }

    #[test]
    fn should_resolve_nord_light_to_ocean_light_syntect_theme() {
        let theme = resolve_theme(ThemeArg::NordLight);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::Base16OceanLight);
    }

    #[test]
    fn should_resolve_nord_dark_high_contrast_to_nord_syntect_theme() {
        let theme = resolve_theme(ThemeArg::NordDarkHighContrast);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::Nord);
    }

    #[test]
    fn should_resolve_nord_light_high_contrast_to_ocean_light_syntect_theme() {
        let theme = resolve_theme(ThemeArg::NordLightHighContrast);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::Base16OceanLight);
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
    fn should_resolve_gruvbox_dark_to_dark_syntect_theme() {
        let theme = resolve_theme(ThemeArg::GruvboxDark);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::GruvboxDark);
    }

    #[test]
    fn should_resolve_gruvbox_light_to_light_syntect_theme() {
        let theme = resolve_theme(ThemeArg::GruvboxLight);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::GruvboxLight);
    }

    #[test]
    fn should_resolve_ayu_light_to_onehalf_light_syntect_theme() {
        let theme = resolve_theme(ThemeArg::AyuLight);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::OneHalfLight);
    }

    #[test]
    fn should_resolve_onedark_to_onehalf_dark_syntect_theme() {
        let theme = resolve_theme(ThemeArg::Onedark);
        assert_eq!(theme.syntect_theme, EmbeddedThemeName::OneHalfDark);
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
