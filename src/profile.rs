use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static PROFILE_LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();

pub fn enabled() -> bool {
    profile_path().is_some()
}

pub fn time<T, F>(label: &str, f: F) -> T
where
    F: FnOnce() -> T,
{
    if !enabled() {
        return f();
    }

    let started = Instant::now();
    let result = f();
    log(label, started.elapsed());
    result
}

pub fn time_with<T, F, M>(label: &str, f: F, metadata: M) -> T
where
    F: FnOnce() -> T,
    M: FnOnce(&T) -> String,
{
    if !enabled() {
        return f();
    }

    let started = Instant::now();
    let result = f();
    log_with(label, started.elapsed(), metadata(&result));
    result
}

pub fn log(label: &str, duration: Duration) {
    log_with(label, duration, String::new());
}

pub fn log_with(label: &str, duration: Duration, metadata: String) {
    let Some(log) = profile_log() else {
        return;
    };

    let suffix = if metadata.is_empty() {
        String::new()
    } else {
        format!(" ({metadata})")
    };
    if let Ok(mut file) = log.lock() {
        let _ = writeln!(
            file,
            "[tuicr profile] {label}: {:.2}ms{suffix}",
            duration.as_secs_f64() * 1000.0
        );
    }
}

fn profile_log() -> Option<&'static Mutex<File>> {
    PROFILE_LOG
        .get_or_init(|| {
            let path = profile_path()?;
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()?;
            Some(Mutex::new(file))
        })
        .as_ref()
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
        return Some(std::env::temp_dir().join("tuicr-profile.log"));
    }

    Some(PathBuf::from(value.as_ref()))
}
