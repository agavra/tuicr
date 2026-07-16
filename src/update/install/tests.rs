use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

use super::archive::{extract_binary, release_asset_name};
use super::method::{detect_install_method, manager_command};
use super::replace::replace_executable;
use super::source::{package_repository_url, release_asset_url};
use super::*;

#[derive(Default)]
struct MockRuntime {
    responses: HashMap<String, Vec<u8>>,
    commands: RefCell<Vec<(InstallMethod, String, Vec<String>)>>,
    replacement: RefCell<Option<(PathBuf, Vec<u8>)>>,
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
}

fn context(executable: impl Into<PathBuf>) -> UpdateContext {
    UpdateContext {
        executable: executable.into(),
        home: Some(PathBuf::from("/home/alice")),
        cargo_home: None,
        current_version: "1.0.0".to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
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
    let asset_name = release_asset_name(version, os, arch).unwrap();
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
            vec!["upgrade", "agavra/tap/tuicr"],
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
    assert!(manager_command(InstallMethod::Direct).is_none());
}

#[test]
fn installs_an_exact_cargo_version_and_rejects_unsupported_manager_pins() {
    let target = semver::Version::parse("0.9.0").unwrap();
    let cargo_runtime = MockRuntime::default();
    assert_eq!(
        update_version_with_runtime(
            &cargo_runtime,
            context("/home/alice/.cargo/bin/tuicr"),
            &target,
        )
        .unwrap(),
        UpdateOutcome::ManagerCompleted(InstallMethod::Cargo)
    );
    assert_eq!(
        cargo_runtime.commands.into_inner()[0].2,
        ["install", "tuicr", "--version", "0.9.0", "--force"]
    );

    assert!(matches!(
        update_version_with_runtime(
            &MockRuntime::default(),
            context("/opt/homebrew/Cellar/tuicr/1.0.0/bin/tuicr"),
            &target,
        ),
        Err(UpdateError::VersionPinUnsupported(InstallMethod::Homebrew))
    ));
}

#[test]
fn installs_an_exact_direct_version_even_when_it_is_older() {
    let target = semver::Version::parse("0.9.0").unwrap();
    let binary = b"known-good-binary";
    let mut runtime = direct_runtime("0.9.0", "linux", "x86_64", tar_gz("tuicr", binary), true);
    let metadata = runtime.responses.remove(&release_api_url(None)).unwrap();
    runtime
        .responses
        .insert(release_api_url(Some(&target)), metadata);

    assert_eq!(
        update_version_with_runtime(&runtime, context("/home/alice/.local/bin/tuicr"), &target,)
            .unwrap(),
        UpdateOutcome::Updated {
            method: InstallMethod::Direct,
            previous_version: "1.0.0".to_string(),
            new_version: "0.9.0".to_string(),
        }
    );
    assert_eq!(runtime.replacement.into_inner().unwrap().1, binary);

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
    let runtime = direct_runtime(
        "1.1.0",
        "linux",
        "x86_64",
        tar_gz("release/tuicr", binary),
        true,
    );
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
fn downloads_verifies_and_extracts_a_windows_direct_install() {
    let binary = b"new-windows-binary";
    let runtime = direct_runtime(
        "1.1.0",
        "windows",
        "x86_64",
        zip_archive("release/tuicr.exe", binary),
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
        ("linux", "x86_64", "x86_64-unknown-linux-gnu.tar.gz"),
        ("linux", "aarch64", "aarch64-unknown-linux-gnu.tar.gz"),
        ("macos", "x86_64", "x86_64-apple-darwin.tar.gz"),
        ("macos", "aarch64", "aarch64-apple-darwin.tar.gz"),
        ("windows", "x86_64", "x86_64-pc-windows-msvc.zip"),
    ];
    for (os, arch, suffix) in cases {
        assert!(
            release_asset_name("1.2.3", os, arch)
                .unwrap()
                .ends_with(suffix)
        );
    }
    assert!(matches!(
        release_asset_name("1.2.3", "windows", "aarch64"),
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
}

#[test]
fn atomically_replaces_a_direct_binary_and_preserves_executable_mode() {
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("tuicr");
    std::fs::write(&executable, b"old").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o751)).unwrap();
        replace_executable(&executable, b"new").unwrap();
        assert_eq!(
            std::fs::metadata(&executable).unwrap().mode() & 0o777,
            0o751
        );
    }
    #[cfg(windows)]
    replace_executable(&executable, b"new").unwrap();
    assert_eq!(std::fs::read(&executable).unwrap(), b"new");
    assert!(matches!(
        replace_executable(Path::new("/"), b"new"),
        Err(UpdateError::Replace { .. })
    ));
}

#[test]
fn formats_methods_outcomes_commands_and_errors_for_users() {
    for (method, name) in [
        (InstallMethod::Homebrew, "Homebrew"),
        (InstallMethod::Cargo, "Cargo"),
        (InstallMethod::Mise, "Mise"),
        (InstallMethod::Nix, "Nix"),
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
