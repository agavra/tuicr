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

    #[error("VCS command failed: {0}")]
    VcsCommand(String),

    #[error("{0}")]
    Forge(String),

    /// A submit that failed after some comments were already posted. The
    /// message is what the user sees; the receipts let the app lock the posted
    /// comments so a retry does not duplicate them.
    #[error("{message}")]
    PartialSubmit {
        message: String,
        receipts: crate::forge::traits::SubmitReceipts,
    },

    #[error("Unsupported operation: {0}")]
    UnsupportedOperation(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}

pub type Result<T> = std::result::Result<T, TuicrError>;
