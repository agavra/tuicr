use std::fmt;
use std::path::{Component, Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    Homebrew,
    Cargo,
    Mise,
    Nix,
    Direct,
}

impl fmt::Display for InstallMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Homebrew => "Homebrew",
            Self::Cargo => "Cargo",
            Self::Mise => "Mise",
            Self::Nix => "Nix",
            Self::Direct => "direct binary",
        })
    }
}

pub(super) struct ManagerCommand {
    pub program: &'static str,
    pub args: &'static [&'static str],
}

pub(super) fn manager_command(method: InstallMethod) -> Option<ManagerCommand> {
    let (program, args): (_, &'static [&'static str]) = match method {
        InstallMethod::Homebrew => ("brew", &["upgrade", "agavra/tap/tuicr"]),
        InstallMethod::Cargo => ("cargo", &["install", "tuicr", "--force"]),
        InstallMethod::Mise => (
            "mise",
            &["upgrade", "github:agavra/tuicr", "--bump", "--yes"],
        ),
        InstallMethod::Nix => ("nix", &["profile", "upgrade", ".*tuicr.*"]),
        InstallMethod::Direct => return None,
    };
    Some(ManagerCommand { program, args })
}

pub(super) fn detect_install_method(
    executable: &Path,
    home: Option<&Path>,
    cargo_home: Option<&Path>,
) -> InstallMethod {
    let components = normalized_components(executable);

    if components.starts_with(&["nix".to_string(), "store".to_string()]) {
        return InstallMethod::Nix;
    }
    if has_sequence(&components, &["cellar", "tuicr"]) {
        return InstallMethod::Homebrew;
    }
    if components.iter().any(|part| part == "installs")
        && components
            .iter()
            .any(|part| part == "mise" || part.contains("agavra-tuicr"))
    {
        return InstallMethod::Mise;
    }

    let cargo_root = cargo_home
        .map(Path::to_path_buf)
        .or_else(|| home.map(|path| path.join(".cargo")));
    if cargo_root.is_some_and(|root| executable.starts_with(root.join("bin"))) {
        return InstallMethod::Cargo;
    }

    InstallMethod::Direct
}

fn has_sequence(components: &[String], parts: &[&str]) -> bool {
    components
        .windows(parts.len())
        .any(|window| window.iter().map(String::as_str).eq(parts.iter().copied()))
}

fn normalized_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_ascii_lowercase()),
            Component::Prefix(prefix) => Some(
                prefix
                    .as_os_str()
                    .to_string_lossy()
                    .trim_end_matches(['/', '\\'])
                    .to_ascii_lowercase(),
            ),
            Component::RootDir | Component::CurDir | Component::ParentDir => None,
        })
        .collect()
}
