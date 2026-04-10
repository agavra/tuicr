use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use toml::Value;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct CommentTypeConfig {
    pub id: String,
    pub label: Option<String>,
    pub definition: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    pub theme: Option<String>,
    pub theme_dark: Option<String>,
    pub theme_light: Option<String>,
    pub appearance: Option<String>,
    pub comment_types: Option<Vec<CommentTypeConfig>>,
    pub show_file_list: Option<bool>,
    pub diff_view: Option<String>,
    pub wrap: Option<bool>,
    pub export_legend: Option<bool>,
}

/// Known top-level config keys. Used to warn about typos.
const KNOWN_KEYS: &[&str] = &[
    "theme",
    "theme_dark",
    "theme_light",
    "appearance",
    "comment_types",
    "show_file_list",
    "diff_view",
    "wrap",
    "export_legend",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigLoadOutcome {
    pub config: Option<AppConfig>,
    pub warnings: Vec<String>,
}

pub fn config_path() -> Result<PathBuf> {
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);

    config_path_from_parts(xdg_config_home, home, appdata)
}

pub fn config_path_hint() -> &'static str {
    #[cfg(windows)]
    {
        r"%APPDATA%\tuicr\config.toml"
    }

    #[cfg(not(windows))]
    {
        "$XDG_CONFIG_HOME/tuicr/config.toml (default: ~/.config/tuicr/config.toml)"
    }
}

fn config_path_from_parts(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    _appdata: Option<PathBuf>,
) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = _appdata
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| anyhow!("Could not determine APPDATA for config directory"))?;
        return Ok(base.join("tuicr").join("config.toml"));
    }

    #[cfg(not(windows))]
    {
        if let Some(base) = xdg_config_home.filter(|p| !p.as_os_str().is_empty()) {
            return Ok(base.join("tuicr").join("config.toml"));
        }

        let home = home
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| anyhow!("Could not determine HOME for config directory"))?;
        Ok(home.join(".config").join("tuicr").join("config.toml"))
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
        comment_types: table
            .get("comment_types")
            .and_then(|v| parse_comment_types(v, &mut warnings)),
        show_file_list: read_bool(table, "show_file_list", &mut warnings),
        diff_view: read_enum(
            table,
            "diff_view",
            &["unified", "side-by-side"],
            &mut warnings,
        ),
        wrap: read_bool(table, "wrap", &mut warnings),
        export_legend: read_bool(table, "export_legend", &mut warnings),
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
}
