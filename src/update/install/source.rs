use semver::Version;

use super::UpdateError;

pub(super) fn package_repository_url() -> &'static str {
    env!("CARGO_PKG_REPOSITORY")
        .trim_end_matches('/')
        .trim_end_matches(".git")
}

pub(super) fn release_api_url(version: Option<&Version>) -> String {
    let repository = package_repository_url()
        .strip_prefix("https://github.com/")
        .expect("package.repository must be an HTTPS github.com URL");
    let release = version.map_or_else(
        || "latest".to_string(),
        |version| format!("tags/v{version}"),
    );
    format!("https://api.github.com/repos/{repository}/releases/{release}")
}

pub(super) fn release_asset_name(
    version: &str,
    os: &str,
    arch: &str,
    target_env: &str,
) -> Result<String, UpdateError> {
    let target = match (os, arch, target_env) {
        ("linux", "x86_64", "gnu") => "x86_64-unknown-linux-gnu",
        ("linux", "x86_64", "musl") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64", "gnu") => "aarch64-unknown-linux-gnu",
        ("linux", "aarch64", "musl") => "aarch64-unknown-linux-musl",
        ("macos", "x86_64", _) => "x86_64-apple-darwin",
        ("macos", "aarch64", _) => "aarch64-apple-darwin",
        ("windows", "x86_64", _) => "x86_64-pc-windows-msvc",
        _ => {
            return Err(UpdateError::UnsupportedPlatform {
                os: os.to_string(),
                arch: arch.to_string(),
            });
        }
    };
    let extension = if os == "windows" { "zip" } else { "tar.gz" };
    Ok(format!("tuicr-{version}-{target}.{extension}"))
}

pub(super) fn release_asset_url(version: &str, asset_name: &str) -> String {
    format!(
        "{}/releases/download/v{version}/{asset_name}",
        package_repository_url()
    )
}
