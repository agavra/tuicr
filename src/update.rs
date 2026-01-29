//! Version update checking against crates.io

use std::time::Duration;

use ureq::Agent;

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

/// Check for updates from crates.io (3-second timeout)
pub fn check_for_updates() -> UpdateCheckResult {
    let current_version = env!("CARGO_PKG_VERSION");

    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3)))
        .build();
    let agent: Agent = config.into();

    let response = match agent.get("https://crates.io/api/v1/crates/tuicr").call() {
        Ok(resp) => resp,
        Err(e) => return UpdateCheckResult::Failed(format!("Network error: {e}")),
    };

    let body: serde_json::Value = match response.into_body().read_json() {
        Ok(json) => json,
        Err(e) => return UpdateCheckResult::Failed(format!("Failed to parse response: {e}")),
    };

    let latest_version = match body
        .get("crate")
        .and_then(|c| c.get("max_version"))
        .and_then(|v| v.as_str())
    {
        Some(v) => v.to_string(),
        None => return UpdateCheckResult::Failed("Could not find version info".to_string()),
    };

    let update_available = is_newer_version(current_version, &latest_version);
    let is_ahead = is_newer_version(&latest_version, current_version);

    let info = UpdateInfo {
        current_version: current_version.to_string(),
        latest_version: latest_version.clone(),
        update_available,
        is_ahead,
    };

    if info.update_available {
        UpdateCheckResult::UpdateAvailable(info)
    } else if info.is_ahead {
        UpdateCheckResult::AheadOfRelease(info)
    } else {
        UpdateCheckResult::UpToDate(info)
    }
}

/// Simple semver comparison (major.minor.patch)
/// Returns true if latest is newer than current
fn is_newer_version(current: &str, latest: &str) -> bool {
    let parse_version = |s: &str| -> Option<(u32, u32, u32)> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() >= 3 {
            Some((
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            ))
        } else if parts.len() == 2 {
            Some((parts[0].parse().ok()?, parts[1].parse().ok()?, 0))
        } else {
            None
        }
    };

    let Some((cur_major, cur_minor, cur_patch)) = parse_version(current) else {
        return false;
    };
    let Some((lat_major, lat_minor, lat_patch)) = parse_version(latest) else {
        return false;
    };

    (lat_major, lat_minor, lat_patch) > (cur_major, cur_minor, cur_patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_version() {
        assert!(is_newer_version("0.5.0", "0.6.0"));
        assert!(is_newer_version("0.5.0", "1.0.0"));
        assert!(is_newer_version("0.5.0", "0.5.1"));
        assert!(!is_newer_version("0.5.0", "0.5.0"));
        assert!(!is_newer_version("0.6.0", "0.5.0"));
        assert!(!is_newer_version("1.0.0", "0.9.9"));
    }

    #[test]
    fn test_ahead_of_release_detection() {
        // When current > latest, we can detect "ahead of release"
        // by checking is_newer_version(latest, current)
        assert!(is_newer_version("0.4.1", "0.5.0")); // 0.5.0 is ahead of 0.4.1 release
        assert!(is_newer_version("0.4.1", "0.4.2")); // 0.4.2 is ahead of 0.4.1 release
        assert!(!is_newer_version("0.5.0", "0.4.1")); // 0.4.1 is not ahead of 0.5.0
    }
}
