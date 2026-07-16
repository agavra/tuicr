//! Version checks and installed-binary updates.

mod check;
mod install;

pub use check::{UpdateCheckResult, UpdateInfo, check_for_updates};
pub use install::{InstallMethod, UpdateError, UpdateOutcome, update_installed};
