mod archive;
mod method;
mod replace;

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::Agent;

use self::archive::{extract_binary, release_asset_name, release_asset_url};
use self::method::{detect_install_method, manager_command};
use self::replace::replace_executable;
use super::check::is_newer_version;

pub use self::method::InstallMethod;

const RELEASE_API_URL: &str = "https://api.github.com/repos/agavra/tuicr/releases/latest";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    ManagerCompleted(InstallMethod),
    Updated {
        method: InstallMethod,
        previous_version: String,
        new_version: String,
    },
    UpToDate {
        method: InstallMethod,
        version: String,
    },
}

impl fmt::Display for UpdateOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManagerCompleted(method) => write!(f, "Update completed through {method}."),
            Self::Updated {
                method,
                previous_version,
                new_version,
            } => write!(
                f,
                "Updated tuicr from {previous_version} to {new_version} ({method})."
            ),
            Self::UpToDate { method, version } => {
                write!(f, "tuicr {version} is already up to date ({method}).")
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("could not locate the running tuicr executable: {0}")]
    CurrentExecutable(#[source] std::io::Error),
    #[error("failed to fetch update metadata: {0}")]
    Network(String),
    #[error("invalid GitHub release metadata: {0}")]
    ReleaseMetadata(String),
    #[error("tuicr does not publish pre-built binaries for {arch}-{os}")]
    UnsupportedPlatform { os: String, arch: String },
    #[error("release asset '{0}' was not found")]
    MissingAsset(String),
    #[error("release asset '{0}' has no SHA-256 digest; refusing an unverified update")]
    MissingDigest(String),
    #[error("SHA-256 verification failed for '{0}'")]
    Integrity(String),
    #[error("failed to extract '{asset}': {detail}")]
    Archive { asset: String, detail: String },
    #[error("{method} could not update tuicr: {detail}")]
    Manager {
        method: InstallMethod,
        detail: String,
    },
    #[error("could not replace '{path}': {detail}")]
    Replace { path: PathBuf, detail: String },
}

pub fn update_installed() -> Result<UpdateOutcome, UpdateError> {
    update_with_runtime(&SystemRuntime, UpdateContext::current()?)
}

#[derive(Debug)]
struct UpdateContext {
    executable: PathBuf,
    home: Option<PathBuf>,
    cargo_home: Option<PathBuf>,
    current_version: String,
    os: String,
    arch: String,
}

impl UpdateContext {
    fn current() -> Result<Self, UpdateError> {
        Ok(Self {
            executable: std::env::current_exe().map_err(UpdateError::CurrentExecutable)?,
            home: directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()),
            cargo_home: std::env::var_os("CARGO_HOME").map(PathBuf::from),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        })
    }
}

trait UpdateRuntime {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, UpdateError>;
    fn run_command(
        &self,
        method: InstallMethod,
        program: &str,
        args: &[&str],
    ) -> Result<(), UpdateError>;
    fn replace_executable(&self, path: &Path, contents: &[u8]) -> Result<(), UpdateError>;
}

struct SystemRuntime;

impl UpdateRuntime for SystemRuntime {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, UpdateError> {
        let config = Agent::config_builder()
            .timeout_global(Some(DOWNLOAD_TIMEOUT))
            .build();
        let agent: Agent = config.into();
        agent
            .get(url)
            .header("User-Agent", concat!("tuicr/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|error| UpdateError::Network(error.to_string()))?
            .into_body()
            .read_to_vec()
            .map_err(|error| UpdateError::Network(error.to_string()))
    }

    fn run_command(
        &self,
        method: InstallMethod,
        program: &str,
        args: &[&str],
    ) -> Result<(), UpdateError> {
        let rendered = display_command(program, args);
        let status =
            Command::new(program)
                .args(args)
                .status()
                .map_err(|error| UpdateError::Manager {
                    method,
                    detail: format!("failed to run `{rendered}`: {error}"),
                })?;
        status.success().then_some(()).ok_or(UpdateError::Manager {
            method,
            detail: format!("`{rendered}` exited with {status}"),
        })
    }

    fn replace_executable(&self, path: &Path, contents: &[u8]) -> Result<(), UpdateError> {
        replace_executable(path, contents)
    }
}

fn update_with_runtime(
    runtime: &impl UpdateRuntime,
    context: UpdateContext,
) -> Result<UpdateOutcome, UpdateError> {
    let method = detect_install_method(
        &context.executable,
        context.home.as_deref(),
        context.cargo_home.as_deref(),
    );
    if let Some(command) = manager_command(method) {
        runtime.run_command(method, command.program, command.args)?;
        return Ok(UpdateOutcome::ManagerCompleted(method));
    }
    update_direct(runtime, &context, method)
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    digest: Option<String>,
}

fn update_direct(
    runtime: &impl UpdateRuntime,
    context: &UpdateContext,
    method: InstallMethod,
) -> Result<UpdateOutcome, UpdateError> {
    let metadata = runtime.fetch(RELEASE_API_URL)?;
    let release: GitHubRelease = serde_json::from_slice(&metadata)
        .map_err(|error| UpdateError::ReleaseMetadata(error.to_string()))?;
    let latest_version = release.tag_name.trim_start_matches('v');
    if !is_newer_version(&context.current_version, latest_version) {
        return Ok(UpdateOutcome::UpToDate {
            method,
            version: context.current_version.clone(),
        });
    }

    let asset_name = release_asset_name(latest_version, &context.os, &context.arch)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| UpdateError::MissingAsset(asset_name.clone()))?;
    let expected_digest = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or_else(|| UpdateError::MissingDigest(asset.name.clone()))?;
    let archive = runtime.fetch(&release_asset_url(latest_version, &asset.name))?;
    if !format!("{:x}", Sha256::digest(&archive)).eq_ignore_ascii_case(expected_digest) {
        return Err(UpdateError::Integrity(asset.name.clone()));
    }

    let binary = extract_binary(&asset.name, &archive)?;
    runtime.replace_executable(&context.executable, &binary)?;
    Ok(UpdateOutcome::Updated {
        method,
        previous_version: context.current_version.clone(),
        new_version: latest_version.to_string(),
    })
}

fn display_command(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
