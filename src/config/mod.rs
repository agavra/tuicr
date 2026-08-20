use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use toml::Value;

pub const DEFAULT_LEADER_KEY: char = ';';

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct CommentTypeConfig {
    pub id: String,
    pub label: Option<String>,
    pub definition: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ForgeConfig {
    /// Prepend `[TYPE] ` to inline review comment bodies on submit so the
    /// reader can see the comment classification at a glance. Defaults to
    /// `true`; set to `false` to send the raw comment body.
    pub comment_type_prefix: bool,
}

impl Default for ForgeConfig {
    fn default() -> Self {
        Self {
            comment_type_prefix: true,
        }
    }
}

const DEFAULT_EXPORT_INTRO: &str =
    "I reviewed your code and have the following comments. Please address them.";
const DEFAULT_EXPORT_COMMENTS_HEADER: &str = "## Local tuicr Comments";
const DEFAULT_EXPORT_REMOTE_COMMENTS_HEADER: &str = "## Existing GitHub Comments";

/// `[export]` section settings shaping the generated review markdown.
///
/// Every field is optional so "unset" stays distinguishable from "set to the
/// default". That distinction is load-bearing for `legend`, which the older
/// top-level `export_legend` key also feeds: `[export]` may only override it
/// when the section actually names the key.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExportConfig {
    /// Intro line above the comment list. An empty string omits it.
    pub intro: Option<String>,
    /// Whether to emit the `Reviewing <scope>` line.
    pub scope_line: Option<bool>,
    /// Whether to emit the `URL:`/`Head:` lines in pull request mode. Kept
    /// separate from `scope_line` because they carry addressable metadata
    /// rather than framing, so trimming the preamble need not drop them.
    pub pr_metadata: Option<bool>,
    /// Heading above locally authored comments. An empty string omits it.
    pub comments_header: Option<String>,
    /// Heading above unresolved remote threads. An empty string omits it.
    pub remote_comments_header: Option<String>,
    /// Whether to emit the `Comment types:` legend.
    pub legend: Option<bool>,
}

impl ExportConfig {
    pub fn intro(&self) -> &str {
        self.intro.as_deref().unwrap_or(DEFAULT_EXPORT_INTRO)
    }

    pub fn scope_line(&self) -> bool {
        self.scope_line.unwrap_or(true)
    }

    pub fn pr_metadata(&self) -> bool {
        self.pr_metadata.unwrap_or(true)
    }

    pub fn comments_header(&self) -> &str {
        self.comments_header
            .as_deref()
            .unwrap_or(DEFAULT_EXPORT_COMMENTS_HEADER)
    }

    pub fn remote_comments_header(&self) -> &str {
        self.remote_comments_header
            .as_deref()
            .unwrap_or(DEFAULT_EXPORT_REMOTE_COMMENTS_HEADER)
    }

    pub fn legend(&self) -> bool {
        self.legend.unwrap_or(true)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    pub theme: Option<String>,
    pub theme_dark: Option<String>,
    pub theme_light: Option<String>,
    pub appearance: Option<String>,
    pub backend: Option<String>,
    pub comment_types: Option<Vec<CommentTypeConfig>>,
    pub show_file_list: Option<bool>,
    /// Starting width of the file list, as a percentage of the content
    /// area. Clamped to 10-60. Defaults to 20; `<leader>L` / `<leader>H`
    /// adjust at runtime.
    pub file_list_width: Option<usize>,
    /// Whether pull-request CI checks are fetched and shown.
    /// Defaults to false.
    pub show_pr_checks: Option<bool>,
    /// Whether pull-request conversation comments are fetched and shown.
    /// Defaults to true.
    pub show_pr_comments: Option<bool>,
    /// Whether the inline commit selector pane is visible on startup for
    /// multi-commit reviews. Defaults to true; toggle at runtime with
    /// `<leader>s` or `:set commits!`.
    pub show_commits: Option<bool>,
    /// Whether files already marked reviewed appear in the file tree and the
    /// diff. Defaults to true; toggle at runtime with `H` (file tree) or
    /// `:set reviewed!`.
    pub show_reviewed: Option<bool>,
    pub diff_view: Option<String>,
    /// Inline commit selector display order: `"descending"` (newest-first,
    /// the default) or `"ascending"` (oldest-first).
    pub commit_order: Option<String>,
    /// Which commits are selected when a multi-commit review first opens:
    /// `"all"` (the default) or `"oldest"` (only the oldest commit, for a
    /// walk-forward per-commit review).
    pub initial_commit_selection: Option<String>,
    pub ignore_whitespace: Option<bool>,
    pub wrap: Option<bool>,
    pub relative_line_numbers: Option<bool>,
    pub export_legend: Option<bool>,
    pub cursor_line: Option<bool>,
    pub search_highlight: Option<bool>,
    pub mouse: Option<bool>,
    /// Enable vim-style modal editing in the review comment text box. When
    /// unset/false the comment box uses the default emacs/readline bindings.
    pub comment_vim: Option<bool>,
    /// Number of spaces inserted by Tab while typing in the vim comment box.
    /// Defaults to 4 (matching diff tab expansion).
    pub comment_tab_width: Option<usize>,
    pub leader: Option<char>,
    pub transparent_background: Option<bool>,
    pub scroll_offset: Option<usize>,
    pub review_watch_interval_ms: Option<usize>,
    /// Disabled by default, and `0` disables it too. Ignored for
    /// pull-request reviews and `--all-files` mode.
    pub diff_watch_interval_ms: Option<usize>,
    pub no_update_check: Option<bool>,
    /// Render single-file and pristine views in full-width mode by default.
    /// Pristine `--all-files` mode already defaults to true regardless of
    /// this setting. Defaults to false.
    pub single_file_view: Option<bool>,
    /// Display name stamped on comments authored locally in the TUI, and
    /// used as the "viewer" identity for per-author coloring in the comment
    /// pane. Defaults to `"user"` when unset.
    pub username: Option<String>,
    /// `[forge]` section settings. Always present; `None` means "no override"
    /// and downstream code should treat it as `ForgeConfig::default()`.
    pub forge: Option<ForgeConfig>,
    /// `[export]` section settings. `None` means "no override"; downstream
    /// code should treat it as `ExportConfig::default()`.
    pub export: Option<ExportConfig>,
}

impl AppConfig {
    /// Effective export settings, layering `[export]` over the older
    /// top-level `export_legend`.
    ///
    /// `[export]` wins only for keys it actually names, so a section that
    /// sets just `intro` leaves a configured `export_legend` in force
    /// instead of resetting the legend to its default.
    pub fn resolved_export(&self) -> ExportConfig {
        let mut export = self.export.clone().unwrap_or_default();
        if export.legend.is_none() {
            export.legend = self.export_legend;
        }
        export
    }
}

/// Known top-level config keys. Used to warn about typos.
const KNOWN_KEYS: &[&str] = &[
    "theme",
    "theme_dark",
    "theme_light",
    "appearance",
    "backend",
    "comment_types",
    "show_file_list",
    "file_list_width",
    "show_pr_checks",
    "show_pr_comments",
    "show_commits",
    "show_reviewed",
    "diff_view",
    "commit_order",
    "initial_commit_selection",
    "ignore_whitespace",
    "wrap",
    "relative_line_numbers",
    "export_legend",
    "cursor_line",
    "search_highlight",
    "mouse",
    "comment_vim",
    "comment_tab_width",
    "leader",
    "transparent_background",
    "scroll_offset",
    "review_watch_interval_ms",
    "diff_watch_interval_ms",
    "no_update_check",
    "single_file_view",
    "username",
    "forge",
    "export",
];

const FORGE_KNOWN_KEYS: &[&str] = &["comment_type_prefix"];

const EXPORT_KNOWN_KEYS: &[&str] = &[
    "intro",
    "scope_line",
    "pr_metadata",
    "comments_header",
    "remote_comments_header",
    "legend",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigLoadOutcome {
    pub config: Option<AppConfig>,
    pub warnings: Vec<String>,
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn themes_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("themes"))
}

fn config_path_env_parts() -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    (
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("APPDATA").map(PathBuf::from),
    )
}

fn config_dir() -> Result<PathBuf> {
    let (xdg_config_home, home, appdata) = config_path_env_parts();
    config_dir_from_parts(xdg_config_home, home, appdata)
}

#[cfg(test)]
fn config_path_from_parts(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    _appdata: Option<PathBuf>,
) -> Result<PathBuf> {
    Ok(config_dir_from_parts(xdg_config_home, home, _appdata)?.join("config.toml"))
}

#[cfg(test)]
fn themes_dir_from_parts(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    _appdata: Option<PathBuf>,
) -> Result<PathBuf> {
    config_dir_from_parts(xdg_config_home, home, _appdata).map(|dir| dir.join("themes"))
}

fn config_dir_from_parts(
    _xdg_config_home: Option<PathBuf>,
    _home: Option<PathBuf>,
    _appdata: Option<PathBuf>,
) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = _appdata
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| anyhow!("Could not determine APPDATA for config directory"))?;
        return Ok(base.join("tuicr"));
    }

    #[cfg(not(windows))]
    {
        if let Some(base) = _xdg_config_home.filter(|p| !p.as_os_str().is_empty()) {
            return Ok(base.join("tuicr"));
        }

        let home = _home
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| anyhow!("Could not determine HOME for config directory"))?;
        Ok(home.join(".config").join("tuicr"))
    }
}

pub fn load_config() -> Result<ConfigLoadOutcome> {
    let path = config_path()?;
    load_config_from_path(&path)
}

/// Read a string value from the table, pushing a warning if the type is wrong.
fn read_string(table: &toml::Table, key: &str, warnings: &mut Vec<String>) -> Option<String> {
    let val = table.get(key)?;
    if let Some(s) = val.as_str() {
        Some(s.to_string())
    } else {
        warnings.push(format!(
            "Warning: Config key '{key}' must be a string; ignoring value"
        ));
        None
    }
}

/// Read a single-character leader key, pushing a warning if the value is unusable.
fn read_leader(table: &toml::Table, warnings: &mut Vec<String>) -> Option<char> {
    let raw = read_string(table, "leader", warnings)?;
    let mut chars = raw.chars();
    match (chars.next(), chars.next()) {
        (Some(leader), None) => Some(leader),
        _ => {
            warnings.push(
                "Warning: Config key 'leader' must be a single character; ignoring value"
                    .to_string(),
            );
            None
        }
    }
}

/// Read a boolean value from the table, pushing a warning if the type is wrong.
fn read_bool(table: &toml::Table, key: &str, warnings: &mut Vec<String>) -> Option<bool> {
    let val = table.get(key)?;
    if let Some(b) = val.as_bool() {
        Some(b)
    } else {
        warnings.push(format!(
            "Warning: Config key '{key}' must be a boolean; ignoring value"
        ));
        None
    }
}

/// Read a non-negative integer value from the table, pushing a warning if the type is wrong.
fn read_usize(table: &toml::Table, key: &str, warnings: &mut Vec<String>) -> Option<usize> {
    let val = table.get(key)?;
    if let Some(n) = val.as_integer() {
        if n >= 0 {
            Some(n as usize)
        } else {
            warnings.push(format!(
                "Warning: Config key '{key}' must be a non-negative integer; ignoring value"
            ));
            None
        }
    } else {
        warnings.push(format!(
            "Warning: Config key '{key}' must be an integer; got '{}', ignoring",
            val
        ));
        None
    }
}

/// Read a string value constrained to a set of allowed values.
fn read_enum(
    table: &toml::Table,
    key: &str,
    allowed: &[&str],
    warnings: &mut Vec<String>,
) -> Option<String> {
    let raw = read_string(table, key, warnings)?;
    if allowed.contains(&raw.as_str()) {
        Some(raw)
    } else {
        let choices = allowed
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(" or ");
        warnings.push(format!(
            "Warning: Config key '{key}' must be {choices}; got \"{raw}\", ignoring"
        ));
        None
    }
}

fn load_config_from_path(path: &Path) -> Result<ConfigLoadOutcome> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(ConfigLoadOutcome::default()),
        Err(err) => return Err(err.into()),
    };

    let value: Value = toml::from_str(&contents)?;
    let table = value
        .as_table()
        .ok_or_else(|| anyhow!("Config root must be a TOML table"))?;

    let mut warnings = Vec::new();

    let config = AppConfig {
        theme: read_string(table, "theme", &mut warnings),
        theme_dark: read_string(table, "theme_dark", &mut warnings),
        theme_light: read_string(table, "theme_light", &mut warnings),
        appearance: read_string(table, "appearance", &mut warnings),
        backend: read_enum(table, "backend", &["libgit2", "cli"], &mut warnings),
        comment_types: table
            .get("comment_types")
            .and_then(|v| parse_comment_types(v, &mut warnings)),
        show_file_list: read_bool(table, "show_file_list", &mut warnings),
        file_list_width: read_usize(table, "file_list_width", &mut warnings),
        show_pr_checks: read_bool(table, "show_pr_checks", &mut warnings),
        show_pr_comments: read_bool(table, "show_pr_comments", &mut warnings),
        show_commits: read_bool(table, "show_commits", &mut warnings),
        show_reviewed: read_bool(table, "show_reviewed", &mut warnings),
        diff_view: read_enum(
            table,
            "diff_view",
            &["unified", "side-by-side"],
            &mut warnings,
        ),
        relative_line_numbers: read_bool(table, "relative_line_numbers", &mut warnings),
        commit_order: read_enum(
            table,
            "commit_order",
            &["descending", "ascending"],
            &mut warnings,
        ),
        initial_commit_selection: read_enum(
            table,
            "initial_commit_selection",
            &["all", "oldest"],
            &mut warnings,
        ),
        ignore_whitespace: read_bool(table, "ignore_whitespace", &mut warnings),
        wrap: read_bool(table, "wrap", &mut warnings),
        export_legend: read_bool(table, "export_legend", &mut warnings),
        cursor_line: read_bool(table, "cursor_line", &mut warnings),
        search_highlight: read_bool(table, "search_highlight", &mut warnings),
        mouse: read_bool(table, "mouse", &mut warnings),
        comment_vim: read_bool(table, "comment_vim", &mut warnings),
        comment_tab_width: read_usize(table, "comment_tab_width", &mut warnings),
        leader: read_leader(table, &mut warnings),
        transparent_background: read_bool(table, "transparent_background", &mut warnings),
        scroll_offset: read_usize(table, "scroll_offset", &mut warnings),
        review_watch_interval_ms: read_usize(table, "review_watch_interval_ms", &mut warnings),
        diff_watch_interval_ms: read_usize(table, "diff_watch_interval_ms", &mut warnings),
        no_update_check: read_bool(table, "no_update_check", &mut warnings),
        single_file_view: read_bool(table, "single_file_view", &mut warnings),
        username: read_string(table, "username", &mut warnings),
        forge: table
            .get("forge")
            .and_then(|v| parse_forge(v, &mut warnings)),
        export: table
            .get("export")
            .and_then(|v| parse_export(v, &mut warnings)),
    };

    for key in table.keys() {
        if !KNOWN_KEYS.contains(&key.as_str()) {
            warnings.push(format!("Warning: Unknown config key '{key}', ignoring"));
        }
    }

    Ok(ConfigLoadOutcome {
        config: Some(config),
        warnings,
    })
}

/// Parse the `[forge]` section, returning `Some` with overridden values when
/// any of the recognized keys are set and `None` when the section is empty (so
/// downstream consumers can fall back to `ForgeConfig::default()`).
fn parse_forge(value: &Value, warnings: &mut Vec<String>) -> Option<ForgeConfig> {
    let Some(table) = value.as_table() else {
        warnings.push("Warning: Config key 'forge' must be a table; ignoring value".to_string());
        return None;
    };

    for key in table.keys() {
        if !FORGE_KNOWN_KEYS.contains(&key.as_str()) {
            warnings.push(format!(
                "Warning: Unknown config key 'forge.{key}', ignoring"
            ));
        }
    }

    let defaults = ForgeConfig::default();
    let mut cfg = defaults.clone();
    let mut any_override = false;

    if let Some(v) = read_section_bool(table, "forge", "comment_type_prefix", warnings) {
        cfg.comment_type_prefix = v;
        any_override = true;
    }

    if any_override { Some(cfg) } else { None }
}

/// Parse the `[export]` section. Returns `Some` only when at least one
/// recognized key is set, so an absent or empty section leaves every default —
/// and the older top-level `export_legend` — untouched.
fn parse_export(value: &Value, warnings: &mut Vec<String>) -> Option<ExportConfig> {
    let Some(table) = value.as_table() else {
        warnings.push("Warning: Config key 'export' must be a table; ignoring value".to_string());
        return None;
    };

    for key in table.keys() {
        if !EXPORT_KNOWN_KEYS.contains(&key.as_str()) {
            warnings.push(format!(
                "Warning: Unknown config key 'export.{key}', ignoring"
            ));
        }
    }

    let cfg = ExportConfig {
        intro: read_section_string(table, "export", "intro", warnings),
        scope_line: read_section_bool(table, "export", "scope_line", warnings),
        pr_metadata: read_section_bool(table, "export", "pr_metadata", warnings),
        comments_header: read_section_string(table, "export", "comments_header", warnings),
        remote_comments_header: read_section_string(
            table,
            "export",
            "remote_comments_header",
            warnings,
        ),
        legend: read_section_bool(table, "export", "legend", warnings),
    };

    if cfg == ExportConfig::default() {
        None
    } else {
        Some(cfg)
    }
}

/// Like `read_bool`, but emits a `<section>.<key>` qualified warning so the
/// user can locate the misconfigured field.
fn read_section_bool(
    table: &toml::Table,
    section: &str,
    key: &str,
    warnings: &mut Vec<String>,
) -> Option<bool> {
    let val = table.get(key)?;
    if let Some(b) = val.as_bool() {
        Some(b)
    } else {
        warnings.push(format!(
            "Warning: Config key '{section}.{key}' must be a boolean; ignoring value"
        ));
        None
    }
}

/// Like `read_string`, but emits a `<section>.<key>` qualified warning.
fn read_section_string(
    table: &toml::Table,
    section: &str,
    key: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let val = table.get(key)?;
    if let Some(s) = val.as_str() {
        Some(s.to_string())
    } else {
        warnings.push(format!(
            "Warning: Config key '{section}.{key}' must be a string; ignoring value"
        ));
        None
    }
}

fn parse_comment_types(
    value: &Value,
    warnings: &mut Vec<String>,
) -> Option<Vec<CommentTypeConfig>> {
    let Some(items) = value.as_array() else {
        warnings.push(
            "Warning: Config key 'comment_types' must be an array of objects; ignoring value"
                .to_string(),
        );
        return None;
    };

    let mut parsed = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for (index, item) in items.iter().enumerate() {
        let Some(entry) = item.as_table() else {
            warnings.push(format!(
                "Warning: Config key 'comment_types[{index}]' must be an object; ignoring entry"
            ));
            continue;
        };

        for key in entry.keys() {
            if key != "id" && key != "label" && key != "definition" && key != "color" {
                warnings.push(format!(
                    "Warning: Unknown key 'comment_types[{index}].{key}', ignoring"
                ));
            }
        }

        let Some(id_raw) = entry.get("id").and_then(Value::as_str) else {
            warnings.push(format!(
                "Warning: Config key 'comment_types[{index}].id' must be a string; ignoring entry"
            ));
            continue;
        };

        let id = id_raw.trim().to_ascii_lowercase();
        if id.is_empty() {
            warnings.push(format!(
                "Warning: Config key 'comment_types[{index}].id' cannot be empty; ignoring entry"
            ));
            continue;
        }

        if seen_ids.contains(&id) {
            warnings.push(format!(
                "Warning: Duplicate comment type id '{id}' in config; ignoring duplicate entry"
            ));
            continue;
        }

        let label = parse_optional_nonempty_string(entry, "label", index, warnings);
        let definition = parse_optional_nonempty_string(entry, "definition", index, warnings);

        let color = match entry.get("color") {
            None => None,
            Some(raw) => match raw.as_str() {
                Some(text) => {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        warnings.push(format!(
                            "Warning: Config key 'comment_types[{index}].color' cannot be empty; ignoring value"
                        ));
                        None
                    } else if !is_supported_color_value(trimmed) {
                        warnings.push(format!(
                            "Warning: Config key 'comment_types[{index}].color' must be a named color or #RRGGBB; ignoring value"
                        ));
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                None => {
                    warnings.push(format!(
                        "Warning: Config key 'comment_types[{index}].color' must be a string; ignoring value"
                    ));
                    None
                }
            },
        };

        seen_ids.insert(id.clone());
        parsed.push(CommentTypeConfig {
            id,
            label,
            definition,
            color,
        });
    }

    if parsed.is_empty() {
        warnings.push(
            "Warning: Config key 'comment_types' contains no valid entries; using defaults"
                .to_string(),
        );
        None
    } else {
        Some(parsed)
    }
}

/// Parse an optional non-empty string field from a comment_types entry.
fn parse_optional_nonempty_string(
    entry: &toml::Table,
    field: &str,
    index: usize,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let raw = entry.get(field)?;
    match raw.as_str() {
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                warnings.push(format!(
                    "Warning: Config key 'comment_types[{index}].{field}' cannot be empty; ignoring value"
                ));
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None => {
            warnings.push(format!(
                "Warning: Config key 'comment_types[{index}].{field}' must be a string; ignoring value"
            ));
            None
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Helper: write a config file, parse it, and return the outcome.
    fn parse_config(toml_content: &str) -> ConfigLoadOutcome {
        let dir = tempdir().expect("failed to create temp dir");
        let path = dir.path().join("config.toml");
        fs::write(&path, toml_content).expect("failed to write config");
        load_config_from_path(&path).expect("config should parse")
    }

    #[test]
    fn should_return_none_when_config_file_missing() {
        let dir = tempdir().expect("failed to create temp dir");
        let path = dir.path().join("config.toml");
        let outcome = load_config_from_path(&path).expect("missing config should not fail");
        assert_eq!(outcome.config, None);
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_load_theme_from_valid_toml() {
        let outcome = parse_config("theme = \"light\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.theme.as_deref()),
            Some("light")
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_load_theme_variants_and_appearance_from_valid_toml() {
        let outcome = parse_config(
            "theme_dark = \"gruvbox-dark\"\ntheme_light = \"gruvbox-light\"\nappearance = \"system\"\n",
        );
        let cfg = outcome.config.as_ref().unwrap();
        assert_eq!(cfg.theme_dark.as_deref(), Some("gruvbox-dark"));
        assert_eq!(cfg.theme_light.as_deref(), Some("gruvbox-light"));
        assert_eq!(cfg.appearance.as_deref(), Some("system"));
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_parse_backend_option() {
        let cli = parse_config("backend = \"cli\"\n");
        assert_eq!(
            cli.config.as_ref().and_then(|cfg| cfg.backend.as_deref()),
            Some("cli")
        );
        assert!(cli.warnings.is_empty());

        let libgit2 = parse_config("backend = \"libgit2\"\n");
        assert_eq!(
            libgit2
                .config
                .as_ref()
                .and_then(|cfg| cfg.backend.as_deref()),
            Some("libgit2")
        );
        assert!(libgit2.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_invalid_backend_option() {
        let outcome = parse_config("backend = \"gitoxide\"\n");
        assert_eq!(outcome.config, Some(AppConfig::default()));
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'backend' must be \"libgit2\" or \"cli\"; got \"gitoxide\", ignoring"
        );
    }

    #[test]
    fn should_warn_and_ignore_backend_with_invalid_type() {
        let outcome = parse_config("backend = true\n");
        assert_eq!(outcome.config, Some(AppConfig::default()));
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'backend' must be a string; ignoring value"
        );
    }

    #[test]
    fn should_parse_empty_config_as_defaults() {
        let outcome = parse_config("");
        assert_eq!(outcome.config, Some(AppConfig::default()));
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_error_on_invalid_toml() {
        let dir = tempdir().expect("failed to create temp dir");
        let path = dir.path().join("config.toml");
        fs::write(&path, "theme =\n").expect("failed to write config");
        let result = load_config_from_path(&path);
        assert!(result.is_err(), "invalid TOML should return error");
    }

    #[test]
    fn should_warn_on_unknown_keys_and_keep_known_values() {
        let outcome = parse_config("theme = \"light\"\nthemes = \"typo\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.theme.as_deref()),
            Some("light")
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Unknown config key 'themes', ignoring"
        );
    }

    #[test]
    fn should_warn_on_unknown_keys_only_and_use_defaults() {
        let outcome = parse_config("themes = \"typo\"\n");
        assert_eq!(outcome.config, Some(AppConfig::default()));
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Unknown config key 'themes', ignoring"
        );
    }

    #[test]
    fn should_warn_and_ignore_theme_with_invalid_type() {
        let outcome = parse_config("theme = 123\n");
        assert_eq!(outcome.config, Some(AppConfig::default()));
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'theme' must be a string; ignoring value"
        );
    }

    #[test]
    fn should_warn_and_ignore_theme_dark_with_invalid_type() {
        let outcome = parse_config("theme_dark = 123\n");
        assert_eq!(outcome.config, Some(AppConfig::default()));
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'theme_dark' must be a string; ignoring value"
        );
    }

    // show_file_list

    #[test]
    fn should_parse_show_file_list_false() {
        let outcome = parse_config("show_file_list = false\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_file_list),
            Some(false)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_show_file_list_with_invalid_type() {
        let outcome = parse_config("show_file_list = \"no\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_file_list),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
    }

    // show_pr_checks

    #[test]
    fn should_parse_show_pr_checks_false() {
        let outcome = parse_config("show_pr_checks = false\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_pr_checks),
            Some(false)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_show_pr_checks_with_invalid_type() {
        let outcome = parse_config("show_pr_checks = \"no\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_pr_checks),
            None
        );
        assert_eq!(
            outcome.warnings,
            vec!["Warning: Config key 'show_pr_checks' must be a boolean; ignoring value"]
        );
    }

    // show_pr_comments

    #[test]
    fn should_parse_show_pr_comments_false() {
        let outcome = parse_config("show_pr_comments = false\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_pr_comments),
            Some(false)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_show_pr_comments_with_invalid_type() {
        let outcome = parse_config("show_pr_comments = \"no\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_pr_comments),
            None
        );
        assert_eq!(
            outcome.warnings,
            vec!["Warning: Config key 'show_pr_comments' must be a boolean; ignoring value"]
        );
    }

    // show_commits

    #[test]
    fn should_parse_show_commits_false() {
        let outcome = parse_config("show_commits = false\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_commits),
            Some(false)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_show_commits_with_invalid_type() {
        let outcome = parse_config("show_commits = \"no\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_commits),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
    }

    // file_list_width

    #[test]
    fn should_parse_file_list_width() {
        let outcome = parse_config("file_list_width = 35\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.file_list_width),
            Some(35)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_on_non_integer_file_list_width() {
        let outcome = parse_config("file_list_width = \"wide\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.file_list_width),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
    }

    // show_reviewed

    #[test]
    fn should_parse_show_reviewed_false() {
        let outcome = parse_config("show_reviewed = false\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_reviewed),
            Some(false)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_show_reviewed_with_invalid_type() {
        let outcome = parse_config("show_reviewed = \"no\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.show_reviewed),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
    }

    #[test]
    fn should_parse_relative_line_numbers() {
        let outcome = parse_config("relative_line_numbers = true\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.relative_line_numbers),
            Some(true)
        );
        assert!(outcome.warnings.is_empty());
    }

    // diff_view

    #[test]
    fn should_parse_diff_view_side_by_side() {
        let outcome = parse_config("diff_view = \"side-by-side\"\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.diff_view.as_deref()),
            Some("side-by-side")
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_parse_diff_view_unified() {
        let outcome = parse_config("diff_view = \"unified\"\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.diff_view.as_deref()),
            Some("unified")
        );
        assert!(outcome.warnings.is_empty());
    }

    // commit_order / initial_commit_selection

    #[test]
    fn should_parse_commit_order_ascending() {
        let outcome = parse_config("commit_order = \"ascending\"\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.commit_order.as_deref()),
            Some("ascending")
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_commit_order_with_invalid_value() {
        let outcome = parse_config("commit_order = \"sideways\"\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.commit_order.as_deref()),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("\"descending\" or \"ascending\""));
    }

    #[test]
    fn should_parse_initial_commit_selection_oldest() {
        let outcome = parse_config("initial_commit_selection = \"oldest\"\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.initial_commit_selection.as_deref()),
            Some("oldest")
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_initial_commit_selection_with_invalid_value() {
        let outcome = parse_config("initial_commit_selection = \"newest\"\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.initial_commit_selection.as_deref()),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("\"all\" or \"oldest\""));
    }

    #[test]
    fn should_warn_and_ignore_diff_view_with_invalid_value() {
        let outcome = parse_config("diff_view = \"split\"\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.diff_view.as_deref()),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("\"unified\" or \"side-by-side\""));
    }

    #[test]
    fn should_warn_and_ignore_diff_view_with_invalid_type() {
        let outcome = parse_config("diff_view = true\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.diff_view.as_deref()),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'diff_view' must be a string; ignoring value"
        );
    }

    // ignore_whitespace

    #[test]
    fn should_parse_ignore_whitespace_true() {
        let outcome = parse_config("ignore_whitespace = true\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.ignore_whitespace),
            Some(true)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_parse_ignore_whitespace_false() {
        let outcome = parse_config("ignore_whitespace = false\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.ignore_whitespace),
            Some(false)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_ignore_whitespace_with_invalid_type() {
        let outcome = parse_config("ignore_whitespace = \"yes\"\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.ignore_whitespace),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'ignore_whitespace' must be a boolean; ignoring value"
        );
    }

    // wrap

    #[test]
    fn should_parse_wrap_true() {
        let outcome = parse_config("wrap = true\n");
        assert_eq!(outcome.config.as_ref().and_then(|cfg| cfg.wrap), Some(true));
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_parse_wrap_false() {
        let outcome = parse_config("wrap = false\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.wrap),
            Some(false)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_wrap_with_invalid_type() {
        let outcome = parse_config("wrap = \"yes\"\n");
        assert_eq!(outcome.config.as_ref().and_then(|cfg| cfg.wrap), None);
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'wrap' must be a boolean; ignoring value"
        );
    }

    // review_watch_interval_ms

    #[test]
    fn should_parse_review_watch_interval_ms() {
        let outcome = parse_config("review_watch_interval_ms = 250\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.review_watch_interval_ms),
            Some(250)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_parse_zero_review_watch_interval_ms_to_allow_disable() {
        let outcome = parse_config("review_watch_interval_ms = 0\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.review_watch_interval_ms),
            Some(0)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_negative_review_watch_interval_ms() {
        let outcome = parse_config("review_watch_interval_ms = -1\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.review_watch_interval_ms),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'review_watch_interval_ms' must be a non-negative integer; ignoring value"
        );
    }

    // diff_watch_interval_ms

    #[test]
    fn should_parse_diff_watch_interval_ms() {
        let outcome = parse_config("diff_watch_interval_ms = 250\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.diff_watch_interval_ms),
            Some(250)
        );
        assert!(outcome.warnings.is_empty());
    }

    /// Unlike `review_watch_interval_ms`, this feature must default to off:
    /// an absent key must parse to `None`, not a positive interval.
    #[test]
    fn should_default_diff_watch_interval_ms_to_none_when_absent() {
        let outcome = parse_config("theme = \"dark\"\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.diff_watch_interval_ms),
            None
        );
    }

    #[test]
    fn should_parse_zero_diff_watch_interval_ms_to_allow_disable() {
        let outcome = parse_config("diff_watch_interval_ms = 0\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.diff_watch_interval_ms),
            Some(0)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_negative_diff_watch_interval_ms() {
        let outcome = parse_config("diff_watch_interval_ms = -1\n");
        assert_eq!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.diff_watch_interval_ms),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'diff_watch_interval_ms' must be a non-negative integer; ignoring value"
        );
    }

    // mouse

    #[test]
    fn should_parse_mouse_true() {
        let outcome = parse_config("mouse = true\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.mouse),
            Some(true)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_default_mouse_to_none() {
        let outcome = parse_config("\n");
        assert_eq!(outcome.config.as_ref().and_then(|cfg| cfg.mouse), None);
    }

    #[test]
    fn should_warn_and_ignore_mouse_with_invalid_type() {
        let outcome = parse_config("mouse = \"on\"\n");
        assert_eq!(outcome.config.as_ref().and_then(|cfg| cfg.mouse), None);
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'mouse' must be a boolean; ignoring value"
        );
    }

    // leader

    #[test]
    fn should_parse_single_character_leader() {
        let outcome = parse_config("leader = \",\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.leader),
            Some(',')
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_multi_character_leader() {
        let outcome = parse_config("leader = \",,\"\n");
        assert_eq!(outcome.config.as_ref().and_then(|cfg| cfg.leader), None);
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'leader' must be a single character; ignoring value"
        );
    }

    #[test]
    fn should_warn_and_ignore_leader_with_invalid_type() {
        let outcome = parse_config("leader = true\n");
        assert_eq!(outcome.config.as_ref().and_then(|cfg| cfg.leader), None);
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'leader' must be a string; ignoring value"
        );
    }

    // no_update_check

    #[test]
    fn should_parse_no_update_check_true() {
        let outcome = parse_config("no_update_check = true\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.no_update_check),
            Some(true)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_parse_no_update_check_false() {
        let outcome = parse_config("no_update_check = false\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.no_update_check),
            Some(false)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_default_no_update_check_to_none() {
        let outcome = parse_config("\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.no_update_check),
            None
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_no_update_check_with_invalid_type() {
        let outcome = parse_config("no_update_check = \"yes\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.no_update_check),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Config key 'no_update_check' must be a boolean; ignoring value"
        );
    }

    // export_legend

    #[test]
    fn should_parse_export_legend_false() {
        let outcome = parse_config("export_legend = false\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.export_legend),
            Some(false)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_default_export_legend_to_none() {
        let outcome = parse_config("\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.export_legend),
            None
        );
    }

    // scroll_offset

    #[test]
    fn should_parse_scroll_offset() {
        let outcome = parse_config("scroll_offset = 4\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.scroll_offset),
            Some(4)
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_scroll_offset_with_invalid_type() {
        let outcome = parse_config("scroll_offset = \"four\"\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.scroll_offset),
            None
        );
        assert_eq!(outcome.warnings.len(), 1);
    }

    // comment_types

    #[test]
    fn should_parse_comment_types_from_array_of_objects() {
        let outcome = parse_config(
            r#"comment_types = [
  { id = "note", label = "question", definition = "ask for clarification", color = "yellow" },
  { id = "issue" }
]"#,
        );
        let comment_types = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.comment_types.as_ref())
            .expect("comment types should be set");

        assert_eq!(comment_types.len(), 2);
        assert_eq!(comment_types[0].id, "note");
        assert_eq!(comment_types[0].label.as_deref(), Some("question"));
        assert_eq!(
            comment_types[0].definition.as_deref(),
            Some("ask for clarification")
        );
        assert_eq!(comment_types[0].color.as_deref(), Some("yellow"));
        assert_eq!(comment_types[1].id, "issue");
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_and_ignore_invalid_comment_type_entries() {
        let outcome = parse_config(
            r#"comment_types = [
  { id = "" },
  { id = "note" },
  { id = "NOTE" },
  42
]"#,
        );
        let comment_types = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.comment_types.as_ref())
            .expect("comment types should be set");

        assert_eq!(comment_types.len(), 1);
        assert_eq!(comment_types[0].id, "note");
        assert_eq!(outcome.warnings.len(), 3);
    }

    // forge

    #[test]
    fn should_default_forge_to_none_when_section_missing() {
        let outcome = parse_config("");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.forge.clone()),
            None
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_parse_forge_section_overriding_defaults() {
        let outcome = parse_config(
            r#"[forge]
comment_type_prefix = false
"#,
        );
        let forge = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.forge.clone())
            .expect("forge section should parse");
        assert!(!forge.comment_type_prefix);
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_default_forge_to_none_when_section_is_empty_table() {
        // An empty `[forge]` block does not override anything; downstream
        // consumers fall back to defaults.
        let outcome = parse_config("[forge]\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.forge.clone()),
            None
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_on_unknown_forge_keys() {
        let outcome = parse_config(
            r#"[forge]
comment_type_prefix = false
foo = "bar"
"#,
        );
        let forge = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.forge.clone())
            .expect("forge section should parse");
        assert!(!forge.comment_type_prefix);
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            outcome.warnings[0],
            "Warning: Unknown config key 'forge.foo', ignoring"
        );
    }

    #[test]
    fn should_warn_and_ignore_forge_value_with_wrong_type() {
        let outcome = parse_config(
            r#"[forge]
comment_type_prefix = "yes"
"#,
        );
        // Wrong-type fields fall back to defaults; with no other overrides
        // the section is `None`.
        assert!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.forge.clone())
                .is_none()
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert!(
            outcome.warnings[0].contains("forge.comment_type_prefix"),
            "warning should be qualified, got {:?}",
            outcome.warnings[0]
        );
    }

    #[test]
    fn should_warn_when_forge_is_not_a_table() {
        let outcome = parse_config("forge = true\n");
        assert!(
            outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.forge.clone())
                .is_none()
        );
        assert_eq!(
            outcome.warnings,
            vec!["Warning: Config key 'forge' must be a table; ignoring value".to_string()]
        );
    }

    #[test]
    fn forge_defaults_enable_comment_type_prefix() {
        let cfg = ForgeConfig::default();
        assert!(cfg.comment_type_prefix);
    }

    #[test]
    fn should_warn_and_ignore_invalid_comment_type_color() {
        let outcome = parse_config(
            r#"comment_types = [
  { id = "note", color = "not-a-color" }
]"#,
        );
        let comment_types = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.comment_types.as_ref())
            .expect("comment types should be set");

        assert_eq!(comment_types.len(), 1);
        assert_eq!(comment_types[0].id, "note");
        assert_eq!(comment_types[0].color, None);
        assert_eq!(outcome.warnings.len(), 1);
    }

    // export

    #[test]
    fn export_accessors_fall_back_to_shipped_defaults() {
        // Locks the defaults to the strings tuicr has always emitted, so a
        // config-layer change cannot silently alter existing exports.
        let cfg = ExportConfig::default();
        assert_eq!(
            cfg.intro(),
            "I reviewed your code and have the following comments. Please address them."
        );
        assert!(cfg.scope_line());
        assert!(cfg.pr_metadata());
        assert_eq!(cfg.comments_header(), "## Local tuicr Comments");
        assert_eq!(cfg.remote_comments_header(), "## Existing GitHub Comments");
        assert!(cfg.legend());
    }

    #[test]
    fn should_default_export_to_none_when_section_missing() {
        let outcome = parse_config("");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.export.clone()),
            None
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_default_export_to_none_when_section_is_empty_table() {
        let outcome = parse_config("[export]\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.export.clone()),
            None
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_parse_export_section_overriding_defaults() {
        // `r###` because the TOML contains `"##`, which would close `r#"…"#`.
        let outcome = parse_config(
            r###"[export]
intro = "Code review comments:"
scope_line = false
pr_metadata = false
comments_header = "## Comments"
remote_comments_header = "## Upstream"
legend = false
"###,
        );
        let export = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.export.clone())
            .expect("export section should parse");
        assert_eq!(export.intro(), "Code review comments:");
        assert!(!export.scope_line());
        assert!(!export.pr_metadata());
        assert_eq!(export.comments_header(), "## Comments");
        assert_eq!(export.remote_comments_header(), "## Upstream");
        assert!(!export.legend());
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_treat_empty_export_strings_as_explicit_overrides() {
        // An empty string means "omit this line", which is distinct from the
        // key being absent. The accessor must not fall back to the default.
        let outcome = parse_config(
            r#"[export]
intro = ""
comments_header = ""
"#,
        );
        let export = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.export.clone())
            .expect("export section should parse");
        assert_eq!(export.intro(), "");
        assert_eq!(export.comments_header(), "");
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_leave_unset_export_keys_as_none_for_legacy_precedence() {
        // Setting only `intro` must not materialize a `legend` value, or the
        // top-level `export_legend` key would be silently overridden.
        let outcome = parse_config(
            r#"export_legend = false

[export]
intro = "Notes:"
"#,
        );
        let cfg = outcome.config.as_ref().expect("config should parse");
        let export = cfg.export.clone().expect("export section should parse");
        assert_eq!(export.legend, None);
        assert_eq!(cfg.export_legend, Some(false));
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn should_warn_on_unknown_export_keys() {
        let outcome = parse_config(
            r#"[export]
intro = "Notes:"
preamble = "typo"
"#,
        );
        let export = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.export.clone())
            .expect("export section should parse");
        assert_eq!(export.intro(), "Notes:");
        assert_eq!(
            outcome.warnings,
            vec!["Warning: Unknown config key 'export.preamble', ignoring".to_string()]
        );
    }

    #[test]
    fn should_warn_and_ignore_export_string_with_invalid_type() {
        let outcome = parse_config(
            r#"[export]
intro = 42
"#,
        );
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.export.clone()),
            None
        );
        assert_eq!(
            outcome.warnings,
            vec!["Warning: Config key 'export.intro' must be a string; ignoring value".to_string()]
        );
    }

    #[test]
    fn should_warn_and_ignore_export_bool_with_invalid_type() {
        let outcome = parse_config(
            r#"[export]
scope_line = "no"
"#,
        );
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.export.clone()),
            None
        );
        assert_eq!(
            outcome.warnings,
            vec![
                "Warning: Config key 'export.scope_line' must be a boolean; ignoring value"
                    .to_string()
            ]
        );
    }

    #[test]
    fn should_warn_when_export_is_not_a_table() {
        let outcome = parse_config("export = true\n");
        assert_eq!(
            outcome.config.as_ref().and_then(|cfg| cfg.export.clone()),
            None
        );
        assert_eq!(
            outcome.warnings,
            vec!["Warning: Config key 'export' must be a table; ignoring value".to_string()]
        );
    }

    // resolved export precedence

    #[test]
    fn should_default_resolved_export_to_shipped_behavior() {
        let cfg = parse_config("").config.expect("config should parse");
        let export = cfg.resolved_export();
        assert!(export.legend());
        assert!(export.scope_line());
        assert!(export.pr_metadata());
    }

    #[test]
    fn should_resolve_export_legend_from_the_legacy_flat_key() {
        let cfg = parse_config("export_legend = false\n")
            .config
            .expect("config should parse");
        assert!(!cfg.resolved_export().legend());
    }

    #[test]
    fn should_let_export_section_override_the_legacy_legend_key() {
        let cfg = parse_config("export_legend = false\n\n[export]\nlegend = true\n")
            .config
            .expect("config should parse");
        assert!(cfg.resolved_export().legend());
    }

    #[test]
    fn should_keep_legacy_legend_when_export_section_omits_it() {
        // Guards the regression a fully-populated overrides struct would
        // cause: adding `[export]` just to trim the intro must not switch
        // the legend back on.
        let cfg = parse_config("export_legend = false\n\n[export]\nintro = \"\"\n")
            .config
            .expect("config should parse");
        let export = cfg.resolved_export();
        assert!(!export.legend());
        assert_eq!(export.intro(), "");
    }

    // config path resolution

    #[cfg(not(windows))]
    #[test]
    fn should_use_xdg_config_home_when_set() {
        let path = config_path_from_parts(
            Some(PathBuf::from("/tmp/xdg-config")),
            Some(PathBuf::from("/tmp/home")),
            None,
        )
        .expect("config path should resolve");

        assert_eq!(path, PathBuf::from("/tmp/xdg-config/tuicr/config.toml"));
    }

    #[cfg(not(windows))]
    #[test]
    fn should_fallback_to_home_dot_config_when_xdg_unset() {
        let path = config_path_from_parts(None, Some(PathBuf::from("/home/tester")), None)
            .expect("config path should resolve");

        assert_eq!(
            path,
            PathBuf::from("/home/tester/.config/tuicr/config.toml")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn should_ignore_empty_xdg_config_home() {
        let path = config_path_from_parts(
            Some(PathBuf::from("")),
            Some(PathBuf::from("/home/tester")),
            None,
        )
        .expect("config path should resolve");

        assert_eq!(
            path,
            PathBuf::from("/home/tester/.config/tuicr/config.toml")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn should_append_tuicr_config_toml_suffix() {
        let path = config_path_from_parts(
            Some(PathBuf::from("/tmp/xdg-config")),
            Some(PathBuf::from("/tmp/home")),
            None,
        )
        .expect("config path should resolve");

        assert!(path.ends_with(Path::new("tuicr").join("config.toml")));
    }

    #[cfg(not(windows))]
    #[test]
    fn should_use_xdg_themes_dir_when_set() {
        let path = themes_dir_from_parts(
            Some(PathBuf::from("/tmp/xdg-config")),
            Some(PathBuf::from("/tmp/home")),
            None,
        )
        .expect("themes dir should resolve");

        assert_eq!(path, PathBuf::from("/tmp/xdg-config/tuicr/themes"));
    }

    #[cfg(not(windows))]
    #[test]
    fn should_fallback_to_home_dot_config_themes_dir_when_xdg_unset() {
        let path = themes_dir_from_parts(None, Some(PathBuf::from("/home/tester")), None)
            .expect("themes dir should resolve");

        assert_eq!(path, PathBuf::from("/home/tester/.config/tuicr/themes"));
    }

    #[cfg(windows)]
    #[test]
    fn should_use_windows_appdata_base_dir() {
        let path = config_path_from_parts(
            Some(PathBuf::from(r"C:\xdg\ignored")),
            Some(PathBuf::from(r"C:\Users\tester")),
            Some(PathBuf::from(r"C:\Users\tester\AppData\Roaming")),
        )
        .expect("config path should resolve");

        assert_eq!(
            path,
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\tuicr\config.toml")
        );
    }

    #[cfg(windows)]
    #[test]
    fn should_use_windows_appdata_themes_dir() {
        let path = themes_dir_from_parts(
            Some(PathBuf::from(r"C:\xdg\ignored")),
            Some(PathBuf::from(r"C:\Users\tester")),
            Some(PathBuf::from(r"C:\Users\tester\AppData\Roaming")),
        )
        .expect("themes dir should resolve");

        assert_eq!(
            path,
            PathBuf::from(r"C:\Users\tester\AppData\Roaming\tuicr\themes")
        );
    }
}
