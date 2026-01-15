use thiserror::Error;

#[derive(Error, Debug)]
pub enum TuicrError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Not a repository")]
    NotARepository,

    #[error("No changes to review")]
    NoChanges,

    #[error("No comments to export - skipping copy")]
    NoComments,

    #[error("Review session corrupted: {0}")]
    CorruptedSession(String),

    #[error("Clipboard error: {0}")]
    Clipboard(String),

    /// Used by CLI-based VCS backends (hg, jj features)
    #[cfg_attr(not(any(feature = "hg", feature = "jj")), allow(dead_code))]
    #[error("VCS command failed: {0}")]
    VcsCommand(String),

    #[error("Unsupported operation: {0}")]
    UnsupportedOperation(String),
}

pub type Result<T> = std::result::Result<T, TuicrError>;
