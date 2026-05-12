use std::path::PathBuf;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use tracing::field;
use tracing_subscriber::fmt::format::FmtSpan;

static PROFILE_ENABLED: OnceLock<bool> = OnceLock::new();

pub fn init_from_env() {
    let Some(path) = profile_path() else {
        let _ = PROFILE_ENABLED.set(false);
        return;
    };

    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    let _ = std::fs::create_dir_all(&directory);

    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        let _ = PROFILE_ENABLED.set(false);
        return;
    };

    let subscriber = tracing_subscriber::fmt()
        .with_writer(file)
        .with_ansi(false)
        .with_target(false)
        .with_level(false)
        .with_span_events(FmtSpan::CLOSE)
        .finish();

    if tracing::subscriber::set_global_default(subscriber).is_ok() {
        let _ = PROFILE_ENABLED.set(true);
    } else {
        let _ = PROFILE_ENABLED.set(false);
    }
}

pub fn enabled() -> bool {
    *PROFILE_ENABLED.get_or_init(|| profile_path().is_some())
}

pub fn time<T, F>(operation: &'static str, f: F) -> T
where
    F: FnOnce() -> T,
{
    if !enabled() {
        return f();
    }

    let span = tracing::info_span!("tuicr_profile", operation);
    let _enter = span.enter();
    f()
}

pub fn time_with<T, F, M>(operation: &'static str, f: F, metadata: M) -> T
where
    F: FnOnce() -> T,
    M: FnOnce(&T) -> String,
{
    if !enabled() {
        return f();
    }

    let span = tracing::info_span!("tuicr_profile", operation, metadata = field::Empty);
    let _enter = span.enter();
    let result = f();
    span.record("metadata", metadata(&result));
    result
}

fn profile_path() -> Option<PathBuf> {
    let value = std::env::var_os("TUICR_PROFILE")?;
    let value = value.to_string_lossy();
    let normalized = value.trim().to_ascii_lowercase();

    if normalized.is_empty() || matches!(normalized.as_str(), "0" | "false" | "off" | "no") {
        return None;
    }

    if let Some(path) = std::env::var_os("TUICR_PROFILE_FILE") {
        return Some(PathBuf::from(path));
    }

    if matches!(normalized.as_str(), "1" | "true" | "on" | "yes") {
        return Some(std::env::temp_dir().join(default_profile_filename(Utc::now())));
    }

    Some(PathBuf::from(value.as_ref()))
}

fn default_profile_filename(started_at: DateTime<Utc>) -> String {
    format!(
        "tuicr-profile.{}.log",
        started_at.format("%Y%m%dT%H%M%S%.3fZ")
    )
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::default_profile_filename;

    #[test]
    fn default_profile_filename_includes_start_timestamp() {
        let started_at = Utc
            .with_ymd_and_hms(2026, 5, 12, 22, 24, 41)
            .single()
            .expect("valid timestamp");

        assert_eq!(
            default_profile_filename(started_at),
            "tuicr-profile.20260512T222441.000Z.log"
        );
    }
}
