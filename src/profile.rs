use std::time::{Duration, Instant};

pub fn enabled() -> bool {
    std::env::var_os("TUICR_PROFILE").is_some()
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
    if enabled() {
        let suffix = if metadata.is_empty() {
            String::new()
        } else {
            format!(" ({metadata})")
        };
        eprintln!(
            "[tuicr profile] {label}: {:.2}ms{suffix}",
            duration.as_secs_f64() * 1000.0
        );
    }
}
