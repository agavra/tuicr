use std::fmt;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    Homebrew,
    Cargo,
    Mise,
    Nix,
    /// Owned by the Arch Linux package database (official repo or AUR).
    Pacman,
    /// Linked from a Home Manager generation or a declarative per-user profile.
    HomeManager,
    Direct,
}

impl fmt::Display for InstallMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Homebrew => "Homebrew",
            Self::Cargo => "Cargo",
            Self::Mise => "Mise",
            Self::Nix => "Nix",
            Self::Pacman => "pacman",
            Self::HomeManager => "Home Manager",
            Self::Direct => "direct binary",
        })
    }
}

impl InstallMethod {
    /// How to update an installation that `tuicr update` must not touch.
    pub(super) fn external_update_hint(self) -> Option<&'static str> {
        match self {
            Self::Pacman => Some("`sudo pacman -Syu` (or your AUR helper)"),
            Self::HomeManager => Some(
                "`home-manager switch` (or `nixos-rebuild switch` / `darwin-rebuild switch` \
                 when Home Manager runs as a system module)",
            ),
            _ => None,
        }
    }
}

pub(super) struct ManagerCommand {
    pub program: &'static str,
    pub args: Vec<String>,
}

pub(super) fn manager_command(method: InstallMethod, executable: &Path) -> Option<ManagerCommand> {
    let (program, args): (_, Vec<String>) = match method {
        InstallMethod::Homebrew => (
            "brew",
            vec!["upgrade".to_string(), homebrew_formula(executable)],
        ),
        InstallMethod::Cargo => ("cargo", strings(&["install", "tuicr", "--force"])),
        InstallMethod::Mise => (
            "mise",
            strings(&["upgrade", "github:agavra/tuicr", "--bump", "--yes"]),
        ),
        InstallMethod::Nix => ("nix", strings(&["profile", "upgrade", ".*tuicr.*"])),
        InstallMethod::Pacman | InstallMethod::HomeManager | InstallMethod::Direct => return None,
    };
    Some(ManagerCommand { program, args })
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

/// The formula to upgrade, read from the keg's `INSTALL_RECEIPT.json`: the
/// Cellar layout is identical for homebrew-core and a third-party tap, so only
/// the receipt's `source.tap` tells them apart. Core, or anything unreadable,
/// falls back to the bare `tuicr`.
fn homebrew_formula(executable: &Path) -> String {
    let receipt = executable
        .ancestors()
        .find(|keg| {
            let formula = keg.parent();
            let cellar = formula.and_then(Path::parent);
            formula
                .and_then(Path::file_name)
                .is_some_and(|name| name == "tuicr")
                && cellar
                    .and_then(Path::file_name)
                    .is_some_and(|name| name.eq_ignore_ascii_case("cellar"))
        })
        .and_then(|keg| std::fs::read(keg.join("INSTALL_RECEIPT.json")).ok());
    homebrew_formula_from_receipt(receipt.as_deref())
}

pub(super) fn homebrew_formula_from_receipt(receipt: Option<&[u8]>) -> String {
    let tap = receipt
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        .and_then(|value| value["source"]["tap"].as_str().map(str::to_ascii_lowercase));
    match tap {
        Some(tap) if tap != "homebrew/core" && is_tap_name(&tap) => format!("{tap}/tuicr"),
        _ => "tuicr".to_string(),
    }
}

fn is_tap_name(tap: &str) -> bool {
    let mut parts = tap.split('/');
    let valid = |part: Option<&str>| {
        part.is_some_and(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    };
    valid(parts.next()) && valid(parts.next()) && parts.next().is_none()
}

/// Installs owned by a system or declarative manager that `tuicr update` must
/// refuse. `root` is `/` in production; tests point it at a temp tree.
pub(super) fn detect_external_manager(
    executable: &Path,
    home: Option<&Path>,
    user: Option<&str>,
    root: &Path,
) -> Option<InstallMethod> {
    if pacman_owns(executable, root) {
        return Some(InstallMethod::Pacman);
    }
    if home_manager_owns(executable, home, user, root) {
        return Some(InstallMethod::HomeManager);
    }
    None
}

/// Mirrors `pacman -Qo`: some package's `files` list in the local database
/// names the executable. Only paths under `/usr` (not `/usr/local`) qualify.
fn pacman_owns(executable: &Path, root: &Path) -> bool {
    let Ok(relative) = executable.strip_prefix(root) else {
        return false;
    };
    if !relative.starts_with("usr") || relative.starts_with("usr/local") {
        return false;
    }
    let Some(relative) = relative.to_str() else {
        return false;
    };
    let Ok(packages) = std::fs::read_dir(root.join("var/lib/pacman/local")) else {
        return false;
    };
    packages.flatten().any(|package| {
        std::fs::read_to_string(package.path().join("files"))
            .is_ok_and(|files| files.lines().any(|line| line == relative))
    })
}

/// A Nix store executable that a Home Manager generation (or a NixOS /
/// nix-darwin per-user profile) links to. Those are rebuilt declaratively, so
/// `nix profile upgrade` would not touch them.
fn home_manager_owns(
    executable: &Path,
    home: Option<&Path>,
    user: Option<&str>,
    root: &Path,
) -> bool {
    if !executable.starts_with(root.join("nix/store")) {
        return false;
    }
    let generation_bin = Path::new("home-path/bin/tuicr");
    let mut links: Vec<PathBuf> = Vec::new();
    if let Some(home) = home {
        links.push(
            home.join(".local/state/nix/profiles/home-manager")
                .join(generation_bin),
        );
        links.push(
            home.join(".local/state/home-manager/gcroots/current-home")
                .join(generation_bin),
        );
    }
    if let Some(user) = user {
        links.push(
            root.join("nix/var/nix/profiles/per-user")
                .join(user)
                .join("home-manager")
                .join(generation_bin),
        );
        links.push(
            root.join("nix/var/nix/gcroots/per-user")
                .join(user)
                .join("current-home")
                .join(generation_bin),
        );
        links.push(
            root.join("etc/profiles/per-user")
                .join(user)
                .join("bin/tuicr"),
        );
    }
    links
        .iter()
        .any(|link| std::fs::canonicalize(link).is_ok_and(|target| target == executable))
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
