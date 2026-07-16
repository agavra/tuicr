use semver::Version;

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

pub(super) fn release_asset_url(version: &str, asset_name: &str) -> String {
    format!(
        "{}/releases/download/v{version}/{asset_name}",
        package_repository_url()
    )
}
