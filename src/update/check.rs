use std::time::Duration;

use ureq::Agent;

const CRATES_IO_API_BASE: &str = "https://crates.io/api/v1/crates";
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub is_ahead: bool,
}

pub enum UpdateCheckResult {
    UpdateAvailable(UpdateInfo),
    UpToDate(UpdateInfo),
    AheadOfRelease(UpdateInfo),
    Failed(String),
}

pub fn check_for_updates() -> UpdateCheckResult {
    let config = Agent::config_builder()
        .timeout_global(Some(CHECK_TIMEOUT))
        .build();
    let agent: Agent = config.into();
    let crates_io_url = crates_io_url();
    let response = match agent.get(&crates_io_url).call() {
        Ok(response) => response,
        Err(error) => return UpdateCheckResult::Failed(format!("Network error: {error}")),
    };
    let body: serde_json::Value = match response.into_body().read_json() {
        Ok(body) => body,
        Err(error) => {
            return UpdateCheckResult::Failed(format!("Failed to parse response: {error}"));
        }
    };

    classify_versions(env!("CARGO_PKG_VERSION"), latest_version(&body))
}

fn crates_io_url() -> String {
    format!("{CRATES_IO_API_BASE}/{}", env!("CARGO_PKG_NAME"))
}

fn latest_version(body: &serde_json::Value) -> Option<&str> {
    body.get("crate")?.get("max_version")?.as_str()
}

fn classify_versions(current: &str, latest: Option<&str>) -> UpdateCheckResult {
    let Some(latest) = latest else {
        return UpdateCheckResult::Failed("Could not find version info".to_string());
    };
    let update_available = is_newer_version(current, latest);
    let is_ahead = is_newer_version(latest, current);
    let info = UpdateInfo {
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        update_available,
        is_ahead,
    };

    if update_available {
        UpdateCheckResult::UpdateAvailable(info)
    } else if is_ahead {
        UpdateCheckResult::AheadOfRelease(info)
    } else {
        UpdateCheckResult::UpToDate(info)
    }
}

pub(super) fn is_newer_version(current: &str, latest: &str) -> bool {
    semver::Version::parse(latest)
        .ok()
        .zip(semver::Version::parse(current).ok())
        .is_some_and(|(latest_version, current_version)| latest_version > current_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_available_current_and_ahead_versions() {
        assert!(matches!(
            classify_versions("1.0.0", Some("1.1.0")),
            UpdateCheckResult::UpdateAvailable(_)
        ));
        assert!(matches!(
            classify_versions("1.0.0", Some("1.0.0")),
            UpdateCheckResult::UpToDate(_)
        ));
        assert!(matches!(
            classify_versions("1.1.0", Some("1.0.0")),
            UpdateCheckResult::AheadOfRelease(_)
        ));
        assert!(matches!(
            classify_versions("1.0.0", None),
            UpdateCheckResult::Failed(_)
        ));
    }

    #[test]
    fn compares_supported_and_invalid_versions() {
        assert!(is_newer_version("0.5.0", "0.6.0"));
        assert!(is_newer_version("0.5.0", "1.0.0"));
        assert!(is_newer_version("0.5.0", "0.5.1"));
        assert!(is_newer_version("1.0.0-beta.1", "1.0.0"));
        assert!(is_newer_version("1.0.0-beta.1", "1.0.0-beta.2"));
        assert!(!is_newer_version("0.5", "0.5.1"));
        assert!(!is_newer_version("0.5.0", "0.5.0"));
        assert!(!is_newer_version("0.6.0", "0.5.0"));
        assert!(!is_newer_version("dev", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "dev"));
        assert!(!is_newer_version("1", "2"));
    }

    #[test]
    fn builds_crates_io_url_from_package_name() {
        assert_eq!(
            crates_io_url(),
            format!("https://crates.io/api/v1/crates/{}", env!("CARGO_PKG_NAME"))
        );
    }

    #[test]
    fn reads_latest_version_from_crates_io_shape() {
        let body = serde_json::json!({"crate": {"max_version": "1.2.3"}});
        assert_eq!(latest_version(&body), Some("1.2.3"));
        assert_eq!(latest_version(&serde_json::json!({})), None);
    }
}
