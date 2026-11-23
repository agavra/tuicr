use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

use super::archive::extract_binary;
use super::executable_swap::{stage_and_swap, swap_executable};
use super::installation::{
    HOMEBREW_TAP_UNKNOWN, detect_external_manager, detect_install_method,
    homebrew_formula_from_receipt, manager_command, state_home,
};
use super::source::{package_repository_url, release_asset_name, release_asset_url};
use super::*;

#[derive(Default)]
struct MockRuntime {
    responses: HashMap<String, Vec<u8>>,
    commands: RefCell<Vec<(InstallMethod, String, Vec<String>)>>,
    replacement: RefCell<Option<(PathBuf, Vec<u8>)>>,
    warnings: RefCell<Vec<String>>,
    command_error: Option<String>,
    replacement_error: Option<String>,
}

impl UpdateRuntime for MockRuntime {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, UpdateError> {
        self.responses
            .get(url)
            .cloned()
            .ok_or_else(|| UpdateError::Network(format!("no response for {url}")))
    }

    fn run_command(
        &self,
        method: InstallMethod,
        program: &str,
        args: &[&str],
    ) -> Result<(), UpdateError> {
        self.commands.borrow_mut().push((
            method,
            program.to_string(),
            args.iter().map(|arg| (*arg).to_string()).collect(),
        ));
        self.command_error.as_ref().map_or(Ok(()), |detail| {
            Err(UpdateError::Manager {
                method,
                detail: detail.clone(),
            })
        })
    }

    fn replace_executable(&self, path: &Path, contents: &[u8]) -> Result<(), UpdateError> {
        *self.replacement.borrow_mut() = Some((path.to_path_buf(), contents.to_vec()));
        self.replacement_error.as_ref().map_or(Ok(()), |detail| {
            Err(UpdateError::Replace {
                path: path.to_path_buf(),
                detail: detail.clone(),
            })
        })
    }

    fn warn(&self, message: &str) {
        self.warnings.borrow_mut().push(message.to_string());
    }
}

fn context(executable: impl Into<PathBuf>) -> UpdateContext {
    let executable = executable.into();
    let method = detect_install_method(&executable, Some(Path::new("/home/alice")), None);
    UpdateContext {
        executable,
        method,
        current_version: "1.0.0".to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        target_env: "gnu".to_string(),
    }
}

fn tar_gz(binary_name: &str, contents: &[u8]) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, binary_name, contents)
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

fn zip_archive(binary_name: &str, contents: &[u8]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    writer
        .start_file(binary_name, SimpleFileOptions::default())
        .unwrap();
    writer.write_all(contents).unwrap();
    writer.finish().unwrap().into_inner()
}

fn direct_runtime(
    version: &str,
    os: &str,
    arch: &str,
    archive: Vec<u8>,
    include_digest: bool,
) -> MockRuntime {
    direct_runtime_for_target(version, os, arch, "gnu", archive, include_digest)
}

fn direct_runtime_for_target(
    version: &str,
    os: &str,
    arch: &str,
    target_env: &str,
    archive: Vec<u8>,
    include_digest: bool,
) -> MockRuntime {
    let asset_name = release_asset_name(version, os, arch, target_env).unwrap();
    let asset_url = release_asset_url(version, &asset_name);
    let digest = format!("sha256:{:x}", Sha256::digest(&archive));
    let metadata = serde_json::json!({
        "tag_name": format!("v{version}"),
        "assets": [{
            "name": asset_name,
            "digest": include_digest.then_some(digest),
        }],
    });
    MockRuntime {
        responses: HashMap::from([
            (
                release_api_url(None),
                serde_json::to_vec(&metadata).unwrap(),
            ),
            (asset_url, archive),
        ]),
        ..MockRuntime::default()
    }
}

#[test]
fn detects_every_documented_install_method_and_custom_cargo_home() {
    let home = Path::new("/home/alice");
    let cases = [
        (
            "/opt/homebrew/Cellar/tuicr/0.19.1/bin/tuicr",
            None,
            InstallMethod::Homebrew,
        ),
        ("/home/alice/.cargo/bin/tuicr", None, InstallMethod::Cargo),
        (
            "/tools/cargo/bin/tuicr",
            Some(Path::new("/tools/cargo")),
            InstallMethod::Cargo,
        ),
        (
            "/home/alice/.local/share/mise/installs/github-agavra-tuicr/0.19.1/tuicr",
            None,
            InstallMethod::Mise,
        ),
        (
            "/nix/store/hash-tuicr-0.19.1/bin/tuicr",
            None,
            InstallMethod::Nix,
        ),
        ("/home/alice/.local/bin/tuicr", None, InstallMethod::Direct),
    ];
    for (path, cargo_home, expected) in cases {
        assert_eq!(
            detect_install_method(Path::new(path), Some(home), cargo_home),
            expected,
            "{path}"
        );
    }
    assert_eq!(
        context("/home/alice/.cargo/bin/tuicr").method,
        InstallMethod::Cargo
    );
}

#[cfg(unix)]
#[test]
fn canonicalizes_an_invocation_symlink_before_install_method_detection() {
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("Cellar/tuicr/1.0.0/bin/tuicr");
    let link = temp.path().join("bin/tuicr");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::fs::write(&executable, b"binary").unwrap();
    std::os::unix::fs::symlink(&executable, &link).unwrap();

    let resolved = canonical_executable(&link).unwrap();

    assert_eq!(resolved, std::fs::canonicalize(&executable).unwrap());
    assert_eq!(
        detect_install_method(&resolved, Some(temp.path()), None),
        InstallMethod::Homebrew
    );
}

#[cfg(windows)]
#[test]
fn detects_windows_cargo_mise_and_direct_binary_layouts() {
    let home = Path::new(r"C:\Users\alice");
    let cases = [
        (r"C:\Users\alice\.cargo\bin\tuicr.exe", InstallMethod::Cargo),
        (
            r"C:\Users\alice\AppData\Local\mise\installs\github-agavra-tuicr\1.0.0\tuicr.exe",
            InstallMethod::Mise,
        ),
        (r"C:\Users\alice\bin\tuicr.exe", InstallMethod::Direct),
    ];
    for (path, expected) in cases {
        assert_eq!(
            detect_install_method(Path::new(path), Some(home), None),
            expected
        );
    }
}

#[test]
fn delegates_all_managed_install_methods_to_their_manager() {
    let cases = [
        (
            "/opt/homebrew/Cellar/tuicr/1.0.0/bin/tuicr",
            InstallMethod::Homebrew,
            "brew",
            vec!["upgrade", "tuicr"],
        ),
        (
            "/home/alice/.cargo/bin/tuicr",
            InstallMethod::Cargo,
            "cargo",
            vec!["install", "tuicr", "--force"],
        ),
        (
            "/home/alice/.local/share/mise/installs/github-agavra-tuicr/1.0.0/tuicr",
            InstallMethod::Mise,
            "mise",
            vec!["upgrade", "github:agavra/tuicr", "--bump", "--yes"],
        ),
        (
            "/nix/store/hash-tuicr-1.0.0/bin/tuicr",
            InstallMethod::Nix,
            "nix",
            vec!["profile", "upgrade", ".*tuicr.*"],
        ),
    ];

    for (path, method, program, args) in cases {
        let runtime = MockRuntime::default();
        assert_eq!(
            update_with_runtime(&runtime, context(path)).unwrap(),
            UpdateOutcome::ManagerCompleted(method)
        );
        assert_eq!(
            runtime.commands.into_inner(),
            vec![(
                method,
                program.to_string(),
                args.into_iter().map(str::to_string).collect(),
            )]
        );
    }
    for method in [
        InstallMethod::Direct,
        InstallMethod::Pacman,
        InstallMethod::HomeManager,
        InstallMethod::DeclarativeNix,
    ] {
        assert!(manager_command(method, Path::new("/usr/bin/tuicr")).is_none());
    }
}

#[test]
fn cargo_supports_exact_upgrades_and_rollbacks() {
    for version in ["0.9.0", "1.1.0"] {
        let target = semver::Version::parse(version).unwrap();
        let runtime = MockRuntime::default();
        assert_eq!(
            update_version_with_runtime(
                &runtime,
                context("/home/alice/.cargo/bin/tuicr"),
                &target,
            )
            .unwrap(),
            UpdateOutcome::ManagerCompleted(InstallMethod::Cargo)
        );
        assert_eq!(
            runtime.commands.into_inner()[0].2,
            ["install", "tuicr", "--version", version, "--force"]
        );
    }
}

#[test]
fn homebrew_mise_and_nix_reject_exact_upgrades_and_rollbacks() {
    let cases = [
        (
            "/opt/homebrew/Cellar/tuicr/1.0.0/bin/tuicr",
            InstallMethod::Homebrew,
        ),
        (
            "/home/alice/.local/share/mise/installs/github-agavra-tuicr/1.0.0/tuicr",
            InstallMethod::Mise,
        ),
        ("/nix/store/hash-tuicr-1.0.0/bin/tuicr", InstallMethod::Nix),
    ];
    for (path, method) in cases {
        for version in ["0.9.0", "1.1.0"] {
            let target = semver::Version::parse(version).unwrap();
            assert!(matches!(
                update_version_with_runtime(&MockRuntime::default(), context(path), &target),
                Err(UpdateError::VersionPinUnsupported(actual)) if actual == method
            ));
        }
    }
}

#[test]
fn direct_binaries_support_exact_upgrades_and_rollbacks() {
    let binary = b"known-good-binary";
    for version in ["0.9.0", "1.1.0"] {
        let target = semver::Version::parse(version).unwrap();
        let mut runtime = direct_runtime(version, "linux", "x86_64", tar_gz("tuicr", binary), true);
        let metadata = runtime.responses.remove(&release_api_url(None)).unwrap();
        runtime
            .responses
            .insert(release_api_url(Some(&target)), metadata);

        assert_eq!(
            update_version_with_runtime(
                &runtime,
                context("/home/alice/.local/bin/tuicr"),
                &target,
            )
            .unwrap(),
            UpdateOutcome::Updated {
                method: InstallMethod::Direct,
                previous_version: "1.0.0".to_string(),
                new_version: version.to_string(),
            }
        );
        assert_eq!(runtime.replacement.into_inner().unwrap().1, binary);
    }
}

#[test]
fn exact_direct_install_skips_the_current_version_and_rejects_mismatched_metadata() {
    let binary = b"known-good-binary";
    let current = semver::Version::parse("1.0.0").unwrap();
    let mut runtime = direct_runtime("1.0.0", "linux", "x86_64", tar_gz("tuicr", binary), true);
    let metadata = runtime.responses.remove(&release_api_url(None)).unwrap();
    runtime
        .responses
        .insert(release_api_url(Some(&current)), metadata);
    assert_eq!(
        update_version_with_runtime(&runtime, context("/home/alice/.local/bin/tuicr"), &current,)
            .unwrap(),
        UpdateOutcome::UpToDate {
            method: InstallMethod::Direct,
            version: "1.0.0".to_string(),
        }
    );
    assert!(runtime.replacement.into_inner().is_none());

    let mismatched_target = semver::Version::parse("0.8.0").unwrap();
    let mut mismatch = direct_runtime("0.9.0", "linux", "x86_64", tar_gz("tuicr", binary), true);
    let metadata = mismatch.responses.remove(&release_api_url(None)).unwrap();
    mismatch
        .responses
        .insert(release_api_url(Some(&mismatched_target)), metadata);
    assert!(matches!(
        update_version_with_runtime(
            &mismatch,
            context("/home/alice/.local/bin/tuicr"),
            &mismatched_target,
        ),
        Err(UpdateError::ReleaseMetadata(_))
    ));
}

#[test]
fn returns_manager_failures_without_replacing_the_binary() {
    let runtime = MockRuntime {
        command_error: Some("upgrade failed".to_string()),
        ..MockRuntime::default()
    };
    let error = update_with_runtime(&runtime, context("/home/alice/.cargo/bin/tuicr")).unwrap_err();
    assert!(error.to_string().contains("Cargo could not update tuicr"));
    assert!(runtime.replacement.into_inner().is_none());
}

#[test]
fn downloads_verifies_and_extracts_a_linux_direct_install() {
    let binary = b"new-linux-binary";
    let runtime = direct_runtime("1.1.0", "linux", "x86_64", tar_gz("tuicr", binary), true);
    assert_eq!(
        update_with_runtime(&runtime, context("/home/alice/.local/bin/tuicr")).unwrap(),
        UpdateOutcome::Updated {
            method: InstallMethod::Direct,
            previous_version: "1.0.0".to_string(),
            new_version: "1.1.0".to_string(),
        }
    );
    assert_eq!(
        runtime.replacement.into_inner(),
        Some((
            PathBuf::from("/home/alice/.local/bin/tuicr"),
            binary.to_vec()
        ))
    );
}

#[test]
fn direct_musl_install_downloads_the_musl_asset() {
    let binary = b"new-musl-binary";
    let runtime = direct_runtime_for_target(
        "1.1.0",
        "linux",
        "x86_64",
        "musl",
        tar_gz("tuicr", binary),
        true,
    );
    let mut context = context("/home/alice/.local/bin/tuicr");
    context.target_env = "musl".to_string();

    assert!(matches!(
        update_with_runtime(&runtime, context),
        Ok(UpdateOutcome::Updated { .. })
    ));
    assert_eq!(runtime.replacement.into_inner().unwrap().1, binary);
}

#[test]
fn downloads_verifies_and_extracts_a_windows_direct_install() {
    let binary = b"new-windows-binary";
    let runtime = direct_runtime(
        "1.1.0",
        "windows",
        "x86_64",
        zip_archive("tuicr.exe", binary),
        true,
    );
    let mut context = context("C:/Users/alice/bin/tuicr.exe");
    context.os = "windows".to_string();
    assert!(matches!(
        update_with_runtime(&runtime, context),
        Ok(UpdateOutcome::Updated { .. })
    ));
    assert_eq!(runtime.replacement.into_inner().unwrap().1, binary);
}

#[test]
fn skips_download_when_direct_install_is_current_or_ahead() {
    for latest in ["1.0.0", "0.9.0"] {
        let metadata = serde_json::json!({"tag_name": latest, "assets": []});
        let runtime = MockRuntime {
            responses: HashMap::from([(
                release_api_url(None),
                serde_json::to_vec(&metadata).unwrap(),
            )]),
            ..MockRuntime::default()
        };
        assert_eq!(
            update_with_runtime(&runtime, context("/home/alice/.local/bin/tuicr")).unwrap(),
            UpdateOutcome::UpToDate {
                method: InstallMethod::Direct,
                version: "1.0.0".to_string(),
            }
        );
        assert!(runtime.replacement.into_inner().is_none());
    }
}

#[test]
fn propagates_fetch_extraction_and_replacement_failures() {
    let direct = || context("/home/alice/.local/bin/tuicr");
    assert!(matches!(
        update_with_runtime(&MockRuntime::default(), direct()),
        Err(UpdateError::Network(_))
    ));

    let mut missing_download =
        direct_runtime("1.1.0", "linux", "x86_64", tar_gz("tuicr", b"binary"), true);
    missing_download
        .responses
        .retain(|url, _| url == &release_api_url(None));
    assert!(matches!(
        update_with_runtime(&missing_download, direct()),
        Err(UpdateError::Network(_))
    ));

    let invalid_archive_runtime = direct_runtime(
        "1.1.0",
        "linux",
        "x86_64",
        b"not a tar archive".to_vec(),
        true,
    );
    assert!(matches!(
        update_with_runtime(&invalid_archive_runtime, direct()),
        Err(UpdateError::Archive { .. })
    ));

    let mut replacement_failure =
        direct_runtime("1.1.0", "linux", "x86_64", tar_gz("tuicr", b"binary"), true);
    replacement_failure.replacement_error = Some("read-only directory".to_string());
    assert!(matches!(
        update_with_runtime(&replacement_failure, direct()),
        Err(UpdateError::Replace { .. })
    ));
}

#[test]
fn rejects_bad_release_metadata_assets_and_digests() {
    let direct = || context("/home/alice/.local/bin/tuicr");
    let invalid_metadata = MockRuntime {
        responses: HashMap::from([(release_api_url(None), b"{".to_vec())]),
        ..MockRuntime::default()
    };
    assert!(matches!(
        update_with_runtime(&invalid_metadata, direct()),
        Err(UpdateError::ReleaseMetadata(_))
    ));

    let no_asset = MockRuntime {
        responses: HashMap::from([(
            release_api_url(None),
            br#"{"tag_name":"1.1.0","assets":[]}"#.to_vec(),
        )]),
        ..MockRuntime::default()
    };
    assert!(matches!(
        update_with_runtime(&no_asset, direct()),
        Err(UpdateError::MissingAsset(_))
    ));

    let no_digest = direct_runtime(
        "1.1.0",
        "linux",
        "x86_64",
        tar_gz("tuicr", b"binary"),
        false,
    );
    assert!(matches!(
        update_with_runtime(&no_digest, direct()),
        Err(UpdateError::MissingDigest(_))
    ));

    let mut bad_digest =
        direct_runtime("1.1.0", "linux", "x86_64", tar_gz("tuicr", b"binary"), true);
    let asset_url = bad_digest
        .responses
        .keys()
        .find(|url| url.starts_with("https://github.com/agavra/tuicr/releases/download/"))
        .unwrap()
        .clone();
    bad_digest.responses.insert(asset_url, b"tampered".to_vec());
    assert!(matches!(
        update_with_runtime(&bad_digest, direct()),
        Err(UpdateError::Integrity(_))
    ));
}

#[test]
fn maps_every_published_target_and_rejects_unsupported_targets() {
    assert_eq!(package_repository_url(), env!("CARGO_PKG_REPOSITORY"));
    let cases = [
        ("linux", "x86_64", "gnu", "x86_64-unknown-linux-gnu.tar.gz"),
        (
            "linux",
            "x86_64",
            "musl",
            "x86_64-unknown-linux-musl.tar.gz",
        ),
        (
            "linux",
            "aarch64",
            "gnu",
            "aarch64-unknown-linux-gnu.tar.gz",
        ),
        (
            "linux",
            "aarch64",
            "musl",
            "aarch64-unknown-linux-musl.tar.gz",
        ),
        ("macos", "x86_64", "", "x86_64-apple-darwin.tar.gz"),
        ("macos", "aarch64", "", "aarch64-apple-darwin.tar.gz"),
        ("windows", "x86_64", "msvc", "x86_64-pc-windows-msvc.zip"),
    ];
    for (os, arch, target_env, suffix) in cases {
        assert!(
            release_asset_name("1.2.3", os, arch, target_env)
                .unwrap()
                .ends_with(suffix)
        );
    }
    assert!(matches!(
        release_asset_name("1.2.3", "windows", "aarch64", "msvc"),
        Err(UpdateError::UnsupportedPlatform { .. })
    ));
    assert_eq!(
        release_asset_url("1.2.3", "tuicr.zip"),
        "https://github.com/agavra/tuicr/releases/download/v1.2.3/tuicr.zip"
    );
    assert_eq!(
        release_api_url(Some(&semver::Version::parse("1.2.3").unwrap())),
        "https://api.github.com/repos/agavra/tuicr/releases/tags/v1.2.3"
    );
}

#[test]
fn rejects_invalid_or_binary_less_archives() {
    let cases = [
        ("tuicr.bin", b"data".as_slice()),
        ("tuicr.tar.gz", b"not gzip".as_slice()),
        ("tuicr.zip", b"not zip".as_slice()),
    ];
    for (name, bytes) in cases {
        assert!(matches!(
            extract_binary(name, bytes),
            Err(UpdateError::Archive { .. })
        ));
    }
    assert!(matches!(
        extract_binary("tuicr.tar.gz", &tar_gz("README", b"text")),
        Err(UpdateError::Archive { .. })
    ));
    assert!(matches!(
        extract_binary("tuicr.zip", &zip_archive("README", b"text")),
        Err(UpdateError::Archive { .. })
    ));
    assert!(matches!(
        extract_binary("tuicr.tar.gz", &tar_gz("nested/tuicr", b"binary")),
        Err(UpdateError::Archive { .. })
    ));
    assert!(matches!(
        extract_binary("tuicr.zip", &zip_archive("nested/tuicr.exe", b"binary")),
        Err(UpdateError::Archive { .. })
    ));
}

#[test]
fn stages_a_sibling_executable_and_preserves_permissions() {
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("tuicr");
    std::fs::write(&executable, b"old").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o751)).unwrap();
    }

    stage_and_swap(&executable, b"new", |staged| {
        assert_eq!(staged.parent(), executable.parent());
        assert_eq!(std::fs::read(staged)?, b"new");
        std::fs::remove_file(&executable)?;
        std::fs::rename(staged, &executable)
    })
    .unwrap();

    assert_eq!(std::fs::read(&executable).unwrap(), b"new");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&executable).unwrap().mode() & 0o777,
            0o751
        );
    }
}

#[test]
fn rejects_unsafe_executable_swap_inputs_before_touching_the_target() {
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("tuicr");
    std::fs::write(&executable, b"old").unwrap();
    let called = Cell::new(false);

    let err = stage_and_swap(&executable, b"", |_| {
        called.set(true);
        Ok(())
    })
    .unwrap_err();
    assert!(
        matches!(&err, UpdateError::Replace { detail, .. } if detail == "downloaded executable is empty"),
        "unexpected error for empty contents: {err:?}"
    );
    assert!(!called.get());
    assert_eq!(std::fs::read(&executable).unwrap(), b"old");

    let err = stage_and_swap(Path::new("relative-tuicr"), b"new", |_| Ok(())).unwrap_err();
    assert!(
        matches!(&err, UpdateError::Replace { detail, .. } if detail == "executable path is not absolute"),
        "unexpected error for relative path: {err:?}"
    );

    let missing = temp.path().join("missing");
    let expected_missing = std::fs::symlink_metadata(&missing).unwrap_err().to_string();
    let err = stage_and_swap(&missing, b"new", |_| Ok(())).unwrap_err();
    assert!(
        matches!(&err, UpdateError::Replace { detail, .. } if detail == &expected_missing),
        "unexpected error for missing path: {err:?}"
    );

    let err = stage_and_swap(temp.path(), b"new", |_| Ok(())).unwrap_err();
    assert!(
        matches!(&err, UpdateError::Replace { detail, .. } if detail == "executable path is not a regular file"),
        "unexpected error for directory target: {err:?}"
    );

    let err = swap_executable(&executable, b"new").unwrap_err();
    assert!(
        matches!(&err, UpdateError::Replace { detail, .. } if detail == "refusing to replace an executable other than the running tuicr binary"),
        "unexpected error for non-running executable: {err:?}"
    );
    assert_eq!(std::fs::read(&executable).unwrap(), b"old");
}

#[cfg(unix)]
#[test]
fn rejects_symlink_executable_targets() {
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("tuicr");
    let link = temp.path().join("tuicr-link");
    std::fs::write(&executable, b"old").unwrap();
    std::os::unix::fs::symlink(&executable, &link).unwrap();

    let err = stage_and_swap(&link, b"new", |_| Ok(())).unwrap_err();
    assert!(
        matches!(&err, UpdateError::Replace { detail, .. } if detail == "executable path is a symlink"),
        "unexpected error for symlink target: {err:?}"
    );
    assert_eq!(std::fs::read(&executable).unwrap(), b"old");
}

#[test]
fn cleans_staged_executable_when_swap_fails() {
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("tuicr");
    std::fs::write(&executable, b"old").unwrap();
    let staged_path = RefCell::new(None);

    let result = stage_and_swap(&executable, b"new", |staged| {
        *staged_path.borrow_mut() = Some(staged.to_path_buf());
        Err(std::io::Error::other("swap failed"))
    });

    assert!(matches!(result, Err(UpdateError::Replace { .. })));
    assert!(!staged_path.into_inner().unwrap().exists());
    assert_eq!(std::fs::read(&executable).unwrap(), b"old");
}

#[test]
fn formats_methods_outcomes_commands_and_errors_for_users() {
    for (method, name) in [
        (InstallMethod::Homebrew, "Homebrew"),
        (InstallMethod::Cargo, "Cargo"),
        (InstallMethod::Mise, "Mise"),
        (InstallMethod::Nix, "Nix"),
        (InstallMethod::Pacman, "pacman"),
        (InstallMethod::HomeManager, "Home Manager"),
        (InstallMethod::DeclarativeNix, "a declarative Nix profile"),
        (InstallMethod::Direct, "direct binary"),
    ] {
        assert_eq!(method.to_string(), name);
    }
    assert_eq!(
        UpdateOutcome::ManagerCompleted(InstallMethod::Cargo).to_string(),
        "Update completed through Cargo."
    );
    assert!(
        UpdateOutcome::Updated {
            method: InstallMethod::Direct,
            previous_version: "1.0.0".to_string(),
            new_version: "1.1.0".to_string(),
        }
        .to_string()
        .contains("1.0.0 to 1.1.0")
    );
    assert!(
        UpdateOutcome::UpToDate {
            method: InstallMethod::Direct,
            version: "1.1.0".to_string(),
        }
        .to_string()
        .contains("already up to date")
    );
    assert_eq!(
        display_command("cargo", &["install", "tuicr"]),
        "cargo install tuicr"
    );
}

#[cfg(unix)]
#[test]
fn system_runtime_reports_success_failure_and_missing_manager_commands() {
    let runtime = SystemRuntime;
    runtime
        .run_command(InstallMethod::Direct, "sh", &["-c", "exit 0"])
        .unwrap();
    assert!(matches!(
        runtime.run_command(InstallMethod::Cargo, "sh", &["-c", "exit 7"]),
        Err(UpdateError::Manager { .. })
    ));
    assert!(matches!(
        runtime.run_command(
            InstallMethod::Cargo,
            "tuicr-command-that-does-not-exist",
            &[]
        ),
        Err(UpdateError::Manager { .. })
    ));
}

#[cfg(unix)]
#[test]
fn detects_cargo_install_behind_a_symlinked_cargo_home() {
    let temp = TempDir::new().unwrap();
    let real = temp.path().join("real-cargo");
    let link = temp.path().join("dot-cargo");
    std::fs::create_dir_all(real.join("bin")).unwrap();
    std::fs::write(real.join("bin/tuicr"), b"binary").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let executable = canonical_executable(&link.join("bin/tuicr")).unwrap();
    let root = canonical_cargo_root(None, Some(&link));

    assert_eq!(
        detect_install_method(&executable, None, root.as_deref()),
        InstallMethod::Cargo
    );
    let root = canonical_cargo_root(Some(temp.path()), None);
    assert_eq!(
        root,
        Some(temp.path().join(".cargo")),
        "missing root falls back to raw path"
    );
}

#[cfg(unix)]
#[test]
fn update_context_resolves_cargo_for_symlinked_cargo_home_and_implicit_home() {
    let temp = TempDir::new().unwrap();
    let real = temp.path().join("real");
    let link = temp.path().join("link");
    std::fs::create_dir_all(real.join(".cargo/bin")).unwrap();
    std::fs::write(real.join(".cargo/bin/tuicr"), b"binary").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let exe = real.join(".cargo/bin/tuicr");

    // Explicit CARGO_HOME behind a symlink.
    let explicit = UpdateContext::resolve(&exe, None, Some(&link.join(".cargo"))).unwrap();
    assert_eq!(explicit.method, InstallMethod::Cargo);
    // Implicit ~/.cargo with a symlinked HOME.
    let implicit = UpdateContext::resolve(&exe, Some(&link), None).unwrap();
    assert_eq!(implicit.method, InstallMethod::Cargo);
}

#[cfg(windows)]
#[test]
fn canonical_executable_and_cargo_root_agree_on_windows_verbatim_paths() {
    let temp = TempDir::new().unwrap();
    let bin = temp.path().join(".cargo").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("tuicr.exe");
    std::fs::write(&exe, b"binary").unwrap();

    let canonical = canonical_executable(&exe).unwrap();
    assert!(canonical.to_string_lossy().starts_with(r"\\?\"));
    let context = UpdateContext::resolve(&exe, Some(temp.path()), None).unwrap();
    assert_eq!(context.method, InstallMethod::Cargo);
    assert_eq!(context.executable, canonical);
}

fn write_file(path: &Path, contents: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[test]
fn homebrew_upgrades_the_formula_named_by_the_install_receipt() {
    let cases = [
        (
            Some(r#"{"source":{"tap":"agavra/tap"}}"#),
            "agavra/tap/tuicr",
            false,
        ),
        (
            Some(r#"{"source":{"tap":"homebrew/core"}}"#),
            "tuicr",
            false,
        ),
        (Some(r#"{"source":{}}"#), "tuicr", true),
        (None, "tuicr", true),
    ];
    for (receipt, formula, warns) in cases {
        let temp = TempDir::new().unwrap();
        let keg = temp.path().join("Cellar/tuicr/1.0.0");
        let executable = keg.join("bin/tuicr");
        write_file(&executable, b"binary");
        if let Some(receipt) = receipt {
            write_file(&keg.join("INSTALL_RECEIPT.json"), receipt.as_bytes());
        }

        let runtime = MockRuntime::default();
        assert_eq!(
            update_with_runtime(&runtime, context(executable)).unwrap(),
            UpdateOutcome::ManagerCompleted(InstallMethod::Homebrew)
        );
        assert_eq!(
            runtime.warnings.into_inner(),
            if warns {
                vec![HOMEBREW_TAP_UNKNOWN.to_string()]
            } else {
                Vec::new()
            },
            "{receipt:?}"
        );
        assert_eq!(
            runtime.commands.into_inner()[0].2,
            ["upgrade", formula],
            "{receipt:?}"
        );
    }
    assert!(!HOMEBREW_TAP_UNKNOWN.contains('\n'));
}

#[cfg(unix)]
#[test]
fn homebrew_warns_but_still_upgrades_when_the_receipt_is_unreadable() {
    let temp = TempDir::new().unwrap();
    let keg = temp.path().join("Cellar/tuicr/1.0.0");
    let executable = keg.join("bin/tuicr");
    write_file(&executable, b"binary");
    // A directory where the receipt should be: present, but not readable as a file.
    std::fs::create_dir(keg.join("INSTALL_RECEIPT.json")).unwrap();

    let runtime = MockRuntime::default();
    assert_eq!(
        update_with_runtime(&runtime, context(executable)).unwrap(),
        UpdateOutcome::ManagerCompleted(InstallMethod::Homebrew)
    );
    assert_eq!(runtime.warnings.into_inner(), [HOMEBREW_TAP_UNKNOWN]);
    assert_eq!(runtime.commands.into_inner()[0].2, ["upgrade", "tuicr"]);
}

#[test]
fn homebrew_receipt_without_a_valid_tap_is_undetermined() {
    for receipt in [
        &b"not json"[..],
        br#"{"source":{"tap":42}}"#,
        br#"{"source":{"tap":"no-slash"}}"#,
        br#"{"source":{"tap":"a/b/c"}}"#,
        br#"{"source":{"tap":"evil/tap --force"}}"#,
    ] {
        assert_eq!(homebrew_formula_from_receipt(Some(receipt)), None);
    }
    assert_eq!(homebrew_formula_from_receipt(None), None);
    assert_eq!(
        homebrew_formula_from_receipt(Some(br#"{"source":{"tap":"Homebrew/Core"}}"#)).as_deref(),
        Some("tuicr")
    );
    assert_eq!(
        homebrew_formula_from_receipt(Some(br#"{"source":{"tap":"Agavra/Tap"}}"#)).as_deref(),
        Some("agavra/tap/tuicr")
    );
}

#[cfg(unix)]
#[test]
fn detects_pacman_owned_executables_from_the_local_database() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let executable = root.join("usr/bin/tuicr");
    write_file(&executable, b"binary");
    write_file(
        &root.join("var/lib/pacman/local/glibc-2.41-1/files"),
        b"%FILES%\nusr/\nusr/lib/libc.so.6\n",
    );
    assert_eq!(
        detect_external_manager(&executable, None, None, root),
        None,
        "an unowned /usr/bin binary stays a direct install"
    );

    write_file(
        &root.join("var/lib/pacman/local/tuicr-1.0.0-1/files"),
        b"%FILES%\nusr/\nusr/bin/\nusr/bin/tuicr\n",
    );
    assert_eq!(
        detect_external_manager(&executable, None, None, root),
        Some(InstallMethod::Pacman)
    );

    let local = root.join("usr/local/bin/tuicr");
    write_file(&local, b"binary");
    write_file(
        &root.join("var/lib/pacman/local/odd-1-1/files"),
        b"%FILES%\nusr/local/bin/tuicr\n",
    );
    assert_eq!(detect_external_manager(&local, None, None, root), None);
}

#[cfg(unix)]
#[test]
fn detects_declarative_nix_profiles_but_not_plain_nix_profiles() {
    let temp = TempDir::new().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let home = root.join("home/alice");
    let state = home.join(".local/state");
    let executable = root.join("nix/store/hash-tuicr-1.0.0/bin/tuicr");
    write_file(&executable, b"binary");
    let link = |path: PathBuf| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&executable, &path).unwrap();
    };

    link(home.join(".nix-profile/bin/tuicr"));
    assert_eq!(
        detect_external_manager(&executable, Some(&state), Some("alice"), &root),
        None,
        "a `nix profile install` stays with Nix"
    );

    // `users.users.alice.packages` (or Home Manager's useUserPackages): the
    // per-user profile does not say which, so it is not labelled Home Manager.
    link(root.join("etc/profiles/per-user/alice/bin/tuicr"));
    assert_eq!(
        detect_external_manager(&executable, Some(&state), Some("bob"), &root),
        None,
        "another user's per-user profile does not count"
    );
    assert_eq!(
        detect_external_manager(&executable, Some(&state), Some("alice"), &root),
        Some(InstallMethod::DeclarativeNix)
    );
}

#[cfg(unix)]
#[test]
fn detects_nixos_and_nix_darwin_system_packages() {
    let temp = TempDir::new().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let executable = root.join("nix/store/hash-tuicr-1.0.0/bin/tuicr");
    write_file(&executable, b"binary");
    assert_eq!(
        detect_external_manager(&executable, None, None, &root),
        None
    );

    let link = root.join("run/current-system/sw/bin/tuicr");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&executable, &link).unwrap();
    assert_eq!(
        detect_external_manager(&executable, None, None, &root),
        Some(InstallMethod::DeclarativeNix)
    );
}

#[cfg(unix)]
#[test]
fn detects_home_manager_generations_under_the_xdg_state_home() {
    let temp = TempDir::new().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let executable = root.join("nix/store/hash-tuicr-1.0.0/bin/tuicr");
    write_file(&executable, b"binary");
    let custom_state = root.join("xdg/state");
    let generation = custom_state.join("nix/profiles/home-manager/home-path/bin/tuicr");
    std::fs::create_dir_all(generation.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&executable, &generation).unwrap();

    let home = root.join("home/alice");
    let default_state = state_home(None, Some(&home)).unwrap();
    assert_eq!(
        detect_external_manager(&executable, Some(&default_state), None, &root),
        None,
        "the generation is not under ~/.local/state"
    );
    let xdg_state = state_home(Some(custom_state.as_os_str()), Some(&home)).unwrap();
    assert_eq!(
        detect_external_manager(&executable, Some(&xdg_state), None, &root),
        Some(InstallMethod::HomeManager)
    );
}

#[test]
fn state_home_honours_an_absolute_xdg_state_home() {
    let home = Path::new("/home/alice");
    assert_eq!(
        state_home(None, Some(home)),
        Some(PathBuf::from("/home/alice/.local/state"))
    );
    let xdg = std::env::temp_dir().join("xdg-state");
    assert_eq!(
        state_home(Some(xdg.as_os_str()), Some(home)),
        Some(xdg.clone())
    );
    assert_eq!(state_home(Some(xdg.as_os_str()), None), Some(xdg));
    assert_eq!(
        state_home(Some("relative/state".as_ref()), Some(home)),
        Some(PathBuf::from("/home/alice/.local/state")),
        "the XDG spec says to ignore relative paths"
    );
    assert_eq!(state_home(None, None), None);
}

#[test]
fn refuses_pacman_and_declarative_nix_installs_without_touching_them() {
    for (method, hint) in [
        (InstallMethod::Pacman, "pacman -Syu"),
        (InstallMethod::HomeManager, "home-manager switch"),
        (InstallMethod::DeclarativeNix, "home-manager switch"),
        (InstallMethod::DeclarativeNix, "nixos-rebuild switch"),
        (InstallMethod::DeclarativeNix, "darwin-rebuild switch"),
    ] {
        let mut ctx = context("/usr/bin/tuicr");
        ctx.method = method;
        let runtime = direct_runtime("1.1.0", "linux", "x86_64", tar_gz("tuicr", b"new"), true);
        let error = update_with_runtime(&runtime, ctx).unwrap_err();
        assert!(
            matches!(error, UpdateError::ExternallyManaged { method: actual, .. } if actual == method)
        );
        assert!(error.to_string().contains(hint), "{error}");
        assert!(runtime.commands.into_inner().is_empty());
        assert!(runtime.replacement.into_inner().is_none());

        let mut ctx = context("/usr/bin/tuicr");
        ctx.method = method;
        let target = semver::Version::parse("0.9.0").unwrap();
        assert!(matches!(
            update_version_with_runtime(&MockRuntime::default(), ctx, &target),
            Err(UpdateError::ExternallyManaged { .. })
        ));
    }
}
