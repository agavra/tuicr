//! Bitbucket-specific submit logic.
//!
//! Bitbucket posts comments individually (not atomically like GitHub).
//! Each inline comment becomes a `bkt pr comment <id> --text ... --file ... --to-line/--from-line`.
//! Approve is a separate `bkt pr approve <id>` call.
//! Draft comments use `--pending`.

use crate::forge::bitbucket::bkt::{BktCommandRunner, SystemBktRunner, map_bkt_error};
use crate::forge::submit::{InlineComment, SubmitEvent};

/// Response from a successful Bitbucket review submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BktCreateReviewResponse {
    /// Number of comments posted.
    pub comments_posted: usize,
    /// Whether approval was submitted.
    pub approved: bool,
}

/// Post a single inline comment via `bkt pr comment`.
fn post_comment(
    runner: &dyn BktCommandRunner,
    pr_number: u64,
    comment: &InlineComment,
    pending: bool,
) -> crate::error::Result<()> {
    let mut args: Vec<String> = vec![
        "pr".to_string(),
        "comment".to_string(),
        pr_number.to_string(),
        "--text".to_string(),
        comment.body.clone(),
    ];
    args.extend(["--file".to_string(), comment.path.to_string_lossy().to_string()]);
    if comment.side == crate::forge::submit::GhSide::Left {
        args.extend(["--from-line".to_string(), comment.line.to_string()]);
    } else {
        args.extend(["--to-line".to_string(), comment.line.to_string()]);
    }
    if pending {
        args.push("--pending".to_string());
    }
    runner
        .run(&args)
        .map_err(map_bkt_error)?;
    Ok(())
}

/// Post a general (non-inline) comment via `bkt pr comment`.
fn post_general_comment(
    runner: &dyn BktCommandRunner,
    pr_number: u64,
    body: &str,
    pending: bool,
) -> crate::error::Result<()> {
    let mut args: Vec<String> = vec![
        "pr".to_string(),
        "comment".to_string(),
        pr_number.to_string(),
        "--text".to_string(),
        body.to_string(),
    ];
    if pending {
        args.push("--pending".to_string());
    }
    runner
        .run(&args)
        .map_err(map_bkt_error)?;
    Ok(())
}

/// Approve the PR via `bkt pr approve`.
fn approve_pr(
    runner: &dyn BktCommandRunner,
    pr_number: u64,
) -> crate::error::Result<()> {
    runner
        .run(&vec![
            "pr".to_string(),
            "approve".to_string(),
            pr_number.to_string(),
        ])
        .map_err(map_bkt_error)?;
    Ok(())
}

/// Submit a review on a Bitbucket PR.
///
/// Posts comments sequentially (not atomically), then approves if the event
/// is `Approve`. Returns a minimal response summarizing what was done.
pub fn create_review(
    runner: &dyn BktCommandRunner,
    pr_number: u64,
    event: SubmitEvent,
    body: &str,
    comments: &[InlineComment],
    pending: bool,
) -> crate::error::Result<BktCreateReviewResponse> {
    let mut comments_posted = 0;

    // Post inline comments sequentially.
    for comment in comments {
        post_comment(runner, pr_number, comment, pending)?;
        comments_posted += 1;
    }

    // Post general body as a top-level comment (Bitbucket has no body field
    // on the review endpoint; it's just a general comment).
    if !body.is_empty() {
        post_general_comment(runner, pr_number, body, pending)?;
        comments_posted += 1;
    }

    // Approve if the event is Approve.
    let approved = matches!(event, SubmitEvent::Approve);
    if approved {
        approve_pr(runner, pr_number)?;
    }

    Ok(BktCreateReviewResponse {
        comments_posted,
        approved,
    })
}

/// Build the create-review response for the app layer.
/// Bitbucket doesn't return a review ID or URL, so we synthesize a minimal one.
pub fn build_response(
    pr_number: u64,
    host: &str,
    owner: &str,
    repo: &str,
    result: &BktCreateReviewResponse,
) -> crate::forge::traits::GhCreateReviewResponse {
    let html_url = format!(
        "https://{}/{}/{}/pull-requests/{}#",
        host, owner, repo, pr_number
    );
    crate::forge::traits::GhCreateReviewResponse {
        id: 0, // Bitbucket has no equivalent review ID
        html_url,
        state: if result.approved {
            "APPROVED".to_string()
        } else if result.comments_posted > 0 {
            "COMMENTED".to_string()
        } else {
            "PENDING".to_string()
        },
    }
}

/// System-level entry point: create a review using the real `bkt` CLI.
pub fn create_review_system(
    pr_number: u64,
    event: SubmitEvent,
    body: &str,
    comments: &[InlineComment],
    pending: bool,
) -> crate::error::Result<crate::forge::traits::GhCreateReviewResponse> {
    let runner = SystemBktRunner;
    let result = create_review(&runner, pr_number, event, body, comments, pending)?;
    // We don't have host/owner/repo here; the caller fills those in.
    // Return a minimal response — the caller wraps it with the real URL.
    Ok(crate::forge::traits::GhCreateReviewResponse {
        id: 0,
        html_url: format!("https://bitbucket.org/PR/{}#", pr_number),
        state: if result.approved {
            "APPROVED".to_string()
        } else {
            "COMMENTED".to_string()
        },
    })
}
