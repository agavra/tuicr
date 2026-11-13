use std::collections::HashSet;
use std::fmt::Write;
use std::io::Write as IoWrite;

use arboard::Clipboard;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::app::{CommentTypeDefinition, DiffSource};
use crate::config::ExportConfig;
use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::{
    PrCommentsVisibility, RemoteReviewComment, RemoteReviewThread, filter_threads,
    group_threads_by_path,
};
use crate::model::{
    Comment, CommentType, DiffFile, DiffHunk, DiffLine, LineOrigin, LineRange, LineSide,
    ReviewSession,
};
use crate::slug::short_sha;
/// (file_path, line_range, side, comment_type, content, commit_id, code_block)
type CommentEntry<'a> = (
    String,
    Option<LineRange>,
    Option<LineSide>,
    String,
    &'a str,
    Option<&'a str>,
    Option<String>,
);

/// Diff rows (`+`, `-` or context) shown on either side of the commented lines when
/// `[export] code_context` is on.
const CODE_CONTEXT_LINES: usize = 2;

/// The loaded diff that `[export] code_context` reads commented lines from.
#[derive(Debug, Default)]
pub struct ExportDiff<'a> {
    pub files: &'a [DiffFile],
    /// Whether the inline commit selector narrows the loaded diff to a
    /// strict subset of the review's commits.
    pub subset_loaded: bool,
    /// The one commit the loaded diff shows, when the selection is exactly
    /// one real commit. `None` for a multi-commit selection, the staged or
    /// unstaged rows, or a review without a commit selector.
    pub single_commit: Option<String>,
}

/// Generate markdown content from the review session.
/// Returns the markdown string or an error if there are no comments.
pub fn generate_export_content(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
    export: &ExportConfig,
    remote_threads: &[RemoteReviewThread],
    session_slug: Option<&str>,
    diff: &ExportDiff,
) -> Result<String> {
    // In PR mode it's still useful to export PR identity + remote
    // discussions even if the user has no local drafts. Outside PR mode
    // we keep the existing behavior of erroring when nothing is to say.
    let has_remote = matches!(diff_source, DiffSource::PullRequest(_))
        && !filter_threads(remote_threads, PrCommentsVisibility::Unresolved).is_empty();
    if !session.has_comments() && !has_remote {
        return Err(TuicrError::NoComments);
    }
    Ok(generate_markdown(
        session,
        diff_source,
        comment_types,
        export,
        remote_threads,
        session_slug,
        diff,
    ))
}

pub fn export_to_clipboard(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
    export: &ExportConfig,
    remote_threads: &[RemoteReviewThread],
    session_slug: Option<&str>,
    diff: &ExportDiff,
) -> Result<String> {
    let content = generate_export_content(
        session,
        diff_source,
        comment_types,
        export,
        remote_threads,
        session_slug,
        diff,
    )?;
    let via_terminal = copy_text_to_clipboard(&content)?;
    Ok(if via_terminal {
        "Review copied to clipboard (via terminal)".to_string()
    } else {
        "Review copied to clipboard".to_string()
    })
}

/// Copy arbitrary text to the system clipboard. Returns `Ok(true)` if the
/// terminal-based fallback (tmux/OSC 52) was used, `Ok(false)` if the
/// platform clipboard handled it.
pub fn copy_text_to_clipboard(text: &str) -> Result<bool> {
    // On macOS, pbcopy writes straight to the system pasteboard and works even
    // inside tmux/SSH. OSC 52 (preferred below) instead relies on the outer
    // terminal honoring the escape, which Terminal.app does not, so the copy
    // would only reach the tmux buffer. Prefer pbcopy unconditionally here.
    if cfg!(target_os = "macos") && try_clipboard_cmd("pbcopy", &[], text) {
        return Ok(false);
    }
    if should_prefer_osc52() {
        copy_osc52(text)?;
        return Ok(true);
    }
    if try_copy_via_subprocess(text) {
        return Ok(false);
    }
    match Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
        Ok(_) => Ok(false),
        Err(_) => {
            copy_osc52(text)?;
            Ok(true)
        }
    }
}

/// Try xclip (X11) then wl-copy (Wayland). Returns true if either succeeds.
fn try_copy_via_subprocess(text: &str) -> bool {
    let session = std::env::var("XDG_SESSION_TYPE");
    if session.is_err() {
        // Session not specified
        return false;
    }
    let session = session.unwrap();
    if session == "wayland" {
        try_clipboard_cmd("wl-copy", &[], text)
    } else if session == "x11" {
        try_clipboard_cmd("xclip", &["-selection", "clipboard"], text)
    } else {
        // Session type unsupported
        false
    }
}

/// Try copying via a CLI tool that forks into the background and holds
/// clipboard ownership beyond tuicr's process lifetime. Returns true on success.
fn try_clipboard_cmd(program: &str, args: &[&str], text: &str) -> bool {
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut stdin, text.as_bytes());
    }
    matches!(child.wait(), Ok(s) if s.success())
}

/// Returns true if we should prefer OSC 52 over the system clipboard.
///
/// In tmux, SSH, Zellij, or Rex sessions, native clipboard tools may "succeed"
/// but copy to the server's clipboard instead of the terminal client's.
fn should_prefer_osc52() -> bool {
    std::env::var("TMUX").is_ok()
        || std::env::var("SSH_TTY").is_ok()
        || std::env::var("ZELLIJ").is_ok()
        || std::env::var("REX_SESSION").is_ok()
        || std::env::var("TERM_PROGRAM").is_ok_and(|program| program == "rex")
}

/// Copy text to clipboard using OSC 52 escape sequence.
/// In tmux, raw OSC 52 is intercepted and may not reach the outer terminal.
/// We use `tmux load-buffer -w` which tells tmux to handle the clipboard copy itself.
fn copy_osc52(text: &str) -> Result<()> {
    if std::env::var("TMUX").is_ok() {
        copy_via_tmux(text)
    } else {
        let mut stdout = std::io::stdout().lock();
        write_osc52(&mut stdout, text)
    }
}

/// Copy text to the system clipboard via `tmux load-buffer -w -`.
/// The `-w` flag tells tmux to also forward to the outer terminal's clipboard via OSC 52.
fn copy_via_tmux(text: &str) -> Result<()> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("tmux")
        .args(["load-buffer", "-w", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| TuicrError::Clipboard(format!("Failed to run tmux: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| TuicrError::Clipboard(format!("Failed to write to tmux: {e}")))?;
    }

    let status = child
        .wait()
        .map_err(|e| TuicrError::Clipboard(format!("tmux load-buffer failed: {e}")))?;

    if !status.success() {
        return Err(TuicrError::Clipboard(
            "tmux load-buffer exited with error".to_string(),
        ));
    }

    Ok(())
}

/// Write OSC 52 escape sequence to the given writer.
/// Separated for testability.
fn write_osc52<W: IoWrite>(writer: &mut W, text: &str) -> Result<()> {
    let encoded = BASE64.encode(text);
    write!(writer, "\x1b]52;c;{encoded}\x07")
        .map_err(|e| TuicrError::Clipboard(format!("Failed to write OSC 52: {e}")))?;
    writer
        .flush()
        .map_err(|e| TuicrError::Clipboard(format!("Failed to flush: {e}")))?;
    Ok(())
}

fn review_scope_label(diff_source: &DiffSource) -> String {
    let scope = match diff_source {
        DiffSource::WorkingTree => "working tree changes".to_string(),
        DiffSource::StagedAndUnstaged => "staged + unstaged changes".to_string(),
        DiffSource::Staged => "staged changes".to_string(),
        DiffSource::Unstaged => "unstaged changes".to_string(),
        DiffSource::CommitRange(_) => "selected commit range".to_string(),
        DiffSource::StagedUnstagedAndCommits(_) => {
            "selected commit range + staged/unstaged changes".to_string()
        }
        DiffSource::PullRequest(pr) => format!(
            "pull request {}#{}",
            pr.key.repository.display_name(),
            pr.key.number
        ),
    };

    format!("Review Comment (scope: {scope})")
}

/// The "Reviewing …" banner for non-pull-request scopes. `None` for the
/// working tree, which has never carried one, and for pull requests, whose
/// banner is emitted alongside their metadata.
fn scope_banner(diff_source: &DiffSource) -> Option<String> {
    fn short_ids(commits: &[String]) -> String {
        commits
            .iter()
            .map(|c| &c[..7.min(c.len())])
            .collect::<Vec<_>>()
            .join(", ")
    }

    match diff_source {
        DiffSource::WorkingTree | DiffSource::PullRequest(_) => None,
        DiffSource::Staged => Some("Reviewing staged changes".to_string()),
        DiffSource::Unstaged => Some("Reviewing unstaged changes".to_string()),
        DiffSource::StagedAndUnstaged => Some("Reviewing staged + unstaged changes".to_string()),
        DiffSource::CommitRange(commits) if commits.len() == 1 => Some(format!(
            "Reviewing commit: {}",
            &commits[0][..7.min(commits[0].len())]
        )),
        DiffSource::CommitRange(commits) => {
            Some(format!("Reviewing commits: {}", short_ids(commits)))
        }
        DiffSource::StagedUnstagedAndCommits(commits) => Some(format!(
            "Reviewing staged + unstaged + commits: {}",
            short_ids(commits)
        )),
    }
}

fn generate_markdown(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
    export: &ExportConfig,
    remote_threads: &[RemoteReviewThread],
    session_slug: Option<&str>,
    diff: &ExportDiff,
) -> String {
    let mut md = String::new();

    // The slug names the session an agent should write comments back into.
    // `session_header = false` drops it for agents that read it as noise.
    if let Some(slug) = session_slug.filter(|_| export.session_header()) {
        let _ = writeln!(md, "## Session: {slug}");
        let _ = writeln!(md);
    }

    // Intro for agents. An empty string drops the line and its spacer so the
    // export opens directly on content.
    let intro = export.intro();
    if !intro.is_empty() {
        let _ = writeln!(md, "{intro}");
        let _ = writeln!(md);
    }

    // Scope banner. `scope_line` governs the prose "what am I reviewing" line;
    // `pr_metadata` separately governs the PR URL and head SHA, which an agent
    // needs to fetch context even when the preamble has been trimmed away.
    match diff_source {
        DiffSource::PullRequest(pr) => {
            let mut wrote_any = false;
            if export.scope_line() {
                let _ = writeln!(
                    md,
                    "Reviewing pull request {}#{}: {}",
                    pr.key.repository.display_name(),
                    pr.key.number,
                    pr.title
                );
                wrote_any = true;
            }
            if export.pr_metadata() {
                let _ = writeln!(md, "URL: {}", pr.url);
                let _ = writeln!(md, "Head: {}", pr.key.short_head());
                wrote_any = true;
            }
            if wrote_any {
                let _ = writeln!(md);
            }
        }
        other => {
            if export.scope_line()
                && let Some(banner) = scope_banner(other)
            {
                let _ = writeln!(md, "{banner}");
                let _ = writeln!(md);
            }
        }
    }

    if export.legend() {
        let used_ids = collect_used_comment_type_ids(session);
        // The typeless `None` default never appears in the legend.
        let legend = comment_types
            .iter()
            .filter(|ct| ct.id != CommentType::NONE_ID)
            .filter(|ct| used_ids.is_empty() || used_ids.contains(&ct.id))
            .map(|comment_type| {
                let definition = comment_type
                    .definition
                    .as_deref()
                    .unwrap_or(comment_type.id.as_str());
                format!(
                    "{} ({})",
                    comment_type.label.to_ascii_uppercase(),
                    definition
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        // Omit the line entirely when there are no typed comments to document
        // (e.g. an untyped-only review).
        if !legend.is_empty() {
            let _ = writeln!(md, "Comment types: {legend}");
            let _ = writeln!(md);
        }
    }

    // Session notes/summary
    if let Some(notes) = &session.session_notes {
        let _ = writeln!(md, "Summary: {notes}");
        let _ = writeln!(md);
    }

    // Collect all comments into a flat list
    let mut all_comments: Vec<CommentEntry> = Vec::new();
    let review_comment_location = review_scope_label(diff_source);

    for comment in &session.review_comments {
        all_comments.push((
            review_comment_location.clone(),
            None,
            None,
            export_comment_type_label(&comment.comment_type, comment_types),
            &comment.content,
            None,
            None,
        ));
    }

    // Sort files by path for consistent output
    let mut files: Vec<_> = session.files.iter().collect();
    files.sort_by_key(|(path, _)| path.to_string_lossy().to_string());

    for (path, review) in files {
        let path_str = path.display().to_string();
        let diff_file = export
            .code_context()
            .then(|| diff.files.iter().find(|f| f.display_path() == path))
            .flatten();

        // File comments (no line number)
        for comment in &review.file_comments {
            all_comments.push((
                path_str.clone(),
                None,
                None,
                export_comment_type_label(&comment.comment_type, comment_types),
                &comment.content,
                comment.commit_id.as_deref(),
                None,
            ));
        }

        // Line comments (with line number, sorted)
        let mut line_comments: Vec<_> = review.line_comments.iter().collect();
        line_comments.sort_by_key(|(line, _)| *line);

        for (line, comments) in line_comments {
            for comment in comments {
                // Use comment's line_range if available, otherwise use the key line
                let line_range = comment
                    .line_range
                    .unwrap_or_else(|| LineRange::single(*line));
                let code_block = diff_file
                    .filter(|_| comment_addresses_loaded_diff(comment, diff))
                    .and_then(|file| {
                        code_context_block(file, line_range, comment.side.unwrap_or(LineSide::New))
                    });
                all_comments.push((
                    path_str.clone(),
                    Some(line_range),
                    comment.side,
                    export_comment_type_label(&comment.comment_type, comment_types),
                    &comment.content,
                    comment.commit_id.as_deref(),
                    code_block,
                ));
            }
        }
    }

    // Output numbered list. An empty header drops the heading but still marks
    // the section as written, so the remote-section separator below survives.
    let mut local_section_written = false;
    if !all_comments.is_empty() {
        let header = export.comments_header();
        if !header.is_empty() {
            let _ = writeln!(md, "{header}");
            let _ = writeln!(md);
        }
        local_section_written = true;
    }
    for (i, (file, line_range, side, comment_type, content, commit_id, code_block)) in
        all_comments.iter().enumerate()
    {
        let number = i + 1;
        let location = match (line_range, side) {
            // Range on deleted side (old lines)
            (Some(range), Some(LineSide::Old)) if range.is_single() => {
                format!("`{}:~{}`", file, range.start)
            }
            (Some(range), Some(LineSide::Old)) => {
                format!("`{}:~{}-~{}`", file, range.start, range.end)
            }
            // Range on new/context side
            (Some(range), _) if range.is_single() => {
                format!("`{}:{}`", file, range.start)
            }
            (Some(range), _) => {
                format!("`{}:{}-{}`", file, range.start, range.end)
            }
            // File comment
            (None, _) => format!("`{file}`"),
        };
        // Append the commit short SHA so the LLM knows which commit the
        // comment was made against — crucial for per-commit reviews.
        let commit_suffix = match commit_id {
            Some(sha) => format!(" (commit {})", short_sha(sha)),
            None => String::new(),
        };
        let marker = format!("{number}.");
        let continuation_indent = " ".repeat(marker.len() + 1);
        let mut content_lines = content.split('\n').map(|line| line.trim_end_matches('\r'));
        let first_line = content_lines.next().unwrap_or_default();
        // Untyped (`None`) comments export with no `**[TYPE]**` marker.
        let type_marker = if comment_type.is_empty() {
            String::new()
        } else {
            format!("**[{comment_type}]** ")
        };
        let _ = writeln!(
            md,
            "{marker} {type_marker}{location}{commit_suffix} - {first_line}"
        );
        for line in content_lines {
            let _ = writeln!(md, "{continuation_indent}{line}");
        }
        // Indented to the list marker's width so the fence nests inside
        // this list item rather than ending the list.
        if let Some(block) = code_block {
            for line in block.lines() {
                let _ = writeln!(md, "{continuation_indent}{line}");
            }
        }
    }

    // PR-mode-only: include unresolved remote discussions grouped by file.
    if matches!(diff_source, DiffSource::PullRequest(_)) {
        let unresolved: Vec<&RemoteReviewThread> =
            filter_threads(remote_threads, PrCommentsVisibility::Unresolved);
        if !unresolved.is_empty() {
            if local_section_written {
                let _ = writeln!(md);
            }
            let remote_header = export.remote_comments_header();
            if !remote_header.is_empty() {
                let _ = writeln!(md, "{remote_header}");
                let _ = writeln!(md);
            }

            // Group threads by file to make the export easy to scan.
            let owned_unresolved: Vec<RemoteReviewThread> =
                unresolved.iter().map(|t| (*t).clone()).collect();
            let groups = group_threads_by_path(&owned_unresolved);
            let mut thread_n = 1;
            for (path, threads) in groups {
                let _ = writeln!(md, "### `{path}`");
                let _ = writeln!(md);
                for thread in threads {
                    if let Some(root) = thread.root() {
                        let author = root.author.as_deref().unwrap_or("unknown");
                        let draft = draft_marker(root);
                        let line_marker = thread.line.map(|l| format!(":{l}")).unwrap_or_default();
                        let _ = writeln!(
                            md,
                            "{thread_n}. `{path}{line_marker}` @{author}{draft} - {body}",
                            body = root.body
                        );
                        if !root.url.is_empty() {
                            let _ = writeln!(md, "   <{}>", root.url);
                        }
                        for reply in thread.replies() {
                            let reply_author = reply.author.as_deref().unwrap_or("unknown");
                            let _ = writeln!(
                                md,
                                "   - @{reply_author}{draft} - {body}",
                                draft = draft_marker(reply),
                                body = reply.body
                            );
                        }
                        thread_n += 1;
                    }
                }
                let _ = writeln!(md);
            }
        }
    }

    md
}

/// Whether the loaded diff is exactly the diff a comment was written
/// against, so quoting it cannot show code the comment was not about.
///
/// Stricter than `App::comment_visible_with`, which also shows unscoped
/// comments over a commit subset and stamped comments inside a wider
/// subset. Those render at their line, but the code there may differ from
/// what the reviewer saw, and a reader trusts quoted code.
fn comment_addresses_loaded_diff(comment: &Comment, diff: &ExportDiff) -> bool {
    match &comment.commit_id {
        None => !diff.subset_loaded,
        Some(sha) => diff.single_commit.as_deref() == Some(sha.as_str()),
    }
}

/// A fenced `diff` block holding the commented lines plus up to
/// [`CODE_CONTEXT_LINES`] diff lines either side, clipped to the hunk.
///
/// Each hunk slice opens with its own `@@ -a,b +c,d @@` header, so a reader
/// can number every line and tell the commented ones from their context.
/// Returns `None` when no loaded line falls in the range, for example after
/// the file changed under the comment. Commit messages get no block: their
/// lines are prose, not a diff.
fn code_context_block(file: &DiffFile, range: LineRange, side: LineSide) -> Option<String> {
    if file.is_commit_message {
        return None;
    }
    let lineno = |line: &DiffLine| match side {
        LineSide::Old => line.old_lineno,
        LineSide::New => line.new_lineno,
    };
    let in_range = |line: &DiffLine| lineno(line).is_some_and(|n| range.contains(n));

    let mut body: Vec<String> = Vec::new();
    for hunk in &file.hunks {
        let Some(first) = hunk.lines.iter().position(in_range) else {
            continue;
        };
        let last = hunk.lines.iter().rposition(in_range).unwrap_or(first);
        let lo = first.saturating_sub(CODE_CONTEXT_LINES);
        let hi = (last + CODE_CONTEXT_LINES).min(hunk.lines.len() - 1);
        body.push(slice_header(hunk, lo, hi));
        for line in &hunk.lines[lo..=hi] {
            let prefix = match line.origin {
                LineOrigin::Addition => '+',
                LineOrigin::Deletion => '-',
                LineOrigin::Context => ' ',
            };
            let text = line.raw_content.as_deref().unwrap_or(&line.content);
            body.push(format!("{prefix}{}", text.trim_end_matches(['\n', '\r'])));
        }
    }
    if body.is_empty() {
        return None;
    }

    // A fence longer than any backtick run in the code, so source that
    // contains ``` cannot close the block early.
    let longest_run = body
        .iter()
        .flat_map(|line| line.split(|c| c != '`'))
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest_run.max(2) + 1);
    let mut block = format!("{fence}diff\n");
    for line in body {
        block.push_str(&line);
        block.push('\n');
    }
    block.push_str(&fence);
    Some(block)
}

/// Unified-diff header for `hunk.lines[lo..=hi]`.
fn slice_header(hunk: &DiffHunk, lo: usize, hi: usize) -> String {
    let (old_start, old_count) = side_span(
        hunk,
        lo,
        hi,
        |l| l.old_lineno,
        hunk.old_start,
        hunk.old_count,
    );
    let (new_start, new_count) = side_span(
        hunk,
        lo,
        hi,
        |l| l.new_lineno,
        hunk.new_start,
        hunk.new_count,
    );
    format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@")
}

/// Start line and line count one side of a hunk slice covers. A side with
/// no lines in the slice names the line before it, as unified diffs do.
fn side_span(
    hunk: &DiffHunk,
    lo: usize,
    hi: usize,
    lineno: impl Fn(&DiffLine) -> Option<u32>,
    hunk_start: u32,
    hunk_count: u32,
) -> (u32, u32) {
    let mut numbers = hunk.lines[lo..=hi].iter().filter_map(&lineno);
    match numbers.next() {
        Some(first) => (first, 1 + numbers.count() as u32),
        None => {
            let before = hunk.lines[..lo].iter().rev().find_map(&lineno);
            let start = before.unwrap_or(if hunk_count == 0 {
                hunk_start
            } else {
                hunk_start.saturating_sub(1)
            });
            (start, 0)
        }
    }
}

/// ` (draft)` for a comment only its author can see yet, so a reader of the
/// export does not take it for something already posted.
fn draft_marker(comment: &RemoteReviewComment) -> &'static str {
    if comment.is_draft { " (draft)" } else { "" }
}

fn collect_used_comment_type_ids(session: &ReviewSession) -> HashSet<String> {
    let mut ids = HashSet::new();
    for c in &session.review_comments {
        ids.insert(c.comment_type.id().to_string());
    }
    for review in session.files.values() {
        for c in &review.file_comments {
            ids.insert(c.comment_type.id().to_string());
        }
        for comments in review.line_comments.values() {
            for c in comments {
                ids.insert(c.comment_type.id().to_string());
            }
        }
    }
    ids
}

/// Export label for a comment type. Returns an empty string for
/// [`CommentType::None`] so the caller omits the `**[TYPE]**` marker.
fn export_comment_type_label(
    comment_type: &CommentType,
    comment_types: &[CommentTypeDefinition],
) -> String {
    if comment_type.is_none() {
        return String::new();
    }

    if let Some(definition) = comment_types
        .iter()
        .find(|definition| definition.id == comment_type.id())
    {
        return definition.label.to_ascii_uppercase();
    }

    comment_type.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::CommentTypeDefinition;
    use crate::model::{Comment, CommentType, FileStatus, LineRange, LineSide, SessionDiffSource};
    use std::path::PathBuf;

    /// Export settings with only the legend suppressed.
    fn legend_off() -> ExportConfig {
        ExportConfig {
            legend: Some(false),
            ..Default::default()
        }
    }

    /// A PR session carrying one local draft comment, for exercising the
    /// boundary between the local and remote sections.
    fn pr_session_with_local_draft() -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/lib.rs")) {
            review.add_line_comment(
                10,
                Comment::new(
                    "Local draft".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                ),
            );
        }
        session
    }

    #[test]
    fn should_use_custom_comment_headers() {
        let session = pr_session_with_local_draft();
        let export = ExportConfig {
            comments_header: Some("## Comments".to_string()),
            remote_comments_header: Some("## Upstream".to_string()),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[sample_remote_thread(
                "a",
                "alice",
                "Can this be simpler?",
                42,
                false,
            )],
            None,
            &ExportDiff::default(),
        );

        assert!(markdown.contains("## Comments"));
        assert!(markdown.contains("## Upstream"));
        assert!(!markdown.contains("## Local tuicr Comments"));
        assert!(!markdown.contains("## Existing GitHub Comments"));
    }

    #[test]
    fn should_keep_remote_separator_when_comments_header_is_empty() {
        // Dropping the local heading must not swallow the blank line that
        // separates the numbered list from the remote section.
        let session = pr_session_with_local_draft();
        let export = ExportConfig {
            comments_header: Some(String::new()),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[sample_remote_thread(
                "a",
                "alice",
                "Can this be simpler?",
                42,
                false,
            )],
            None,
            &ExportDiff::default(),
        );

        assert!(!markdown.contains("## Local tuicr Comments"));
        assert!(
            markdown.contains("\n\n## Existing GitHub Comments"),
            "expected a blank line before the remote header in:\n{markdown}"
        );
    }

    #[test]
    fn should_omit_scope_line_when_disabled() {
        let session = create_test_session();
        let export = ExportConfig {
            scope_line: Some(false),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &DiffSource::Unstaged,
            &comment_types(),
            &export,
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(!markdown.contains("Reviewing unstaged changes"));
    }

    #[test]
    fn should_keep_pr_metadata_when_scope_line_disabled() {
        // URL and head SHA are addressable context an agent needs, not
        // preamble framing, so trimming the scope line must not drop them.
        let session = create_test_session();
        let export = ExportConfig {
            scope_line: Some(false),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(!markdown.contains("Reviewing pull request"));
        assert!(markdown.contains("URL: https://github.com/agavra/tuicr/pull/125"));
        assert!(markdown.contains("Head: "));
    }

    #[test]
    fn should_omit_pr_metadata_when_disabled_but_keep_scope_line() {
        let session = create_test_session();
        let export = ExportConfig {
            pr_metadata: Some(false),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(markdown.contains("Reviewing pull request agavra/tuicr#125"));
        assert!(!markdown.contains("URL: "));
        assert!(!markdown.contains("Head: "));
    }

    #[test]
    fn should_omit_pr_banner_and_spacer_when_scope_and_metadata_disabled() {
        let session = create_test_session();
        let export = ExportConfig {
            intro: Some(String::new()),
            scope_line: Some(false),
            pr_metadata: Some(false),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &export,
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(!markdown.contains("Reviewing pull request"));
        assert!(!markdown.contains("URL: "));
        assert!(
            !markdown.starts_with('\n'),
            "the banner's trailing blank line should go with it, got:\n{markdown}"
        );
    }

    #[test]
    fn should_use_custom_export_intro() {
        let session = create_test_session();
        let export = ExportConfig {
            intro: Some("Code review comments:".to_string()),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &export,
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(markdown.contains("Code review comments:"));
        assert!(!markdown.contains("I reviewed your code"));
    }

    #[test]
    fn should_omit_export_intro_and_its_spacer_when_empty() {
        let session = create_test_session();
        let export = ExportConfig {
            intro: Some(String::new()),
            ..Default::default()
        };

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &export,
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(!markdown.contains("I reviewed your code"));
        assert!(
            !markdown.starts_with('\n'),
            "the intro's trailing blank line should go with it, got:\n{markdown}"
        );
    }

    fn comment_types() -> Vec<CommentTypeDefinition> {
        vec![
            CommentTypeDefinition {
                id: "note".to_string(),
                label: "note".to_string(),
                definition: Some("observations".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "suggestion".to_string(),
                label: "suggestion".to_string(),
                definition: Some("improvements".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "issue".to_string(),
                label: "issue".to_string(),
                definition: Some("problems to fix".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "praise".to_string(),
                label: "praise".to_string(),
                definition: Some("positive feedback".to_string()),
                color: None,
            },
        ]
    }

    fn create_test_session() -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        // Add a file comment
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.reviewed = true;
            review.add_file_comment(Comment::new(
                "Consider adding documentation".to_string(),
                CommentType::from_id("suggestion"),
                None,
            ));
            review.add_line_comment(
                42,
                Comment::new(
                    "Magic number should be a constant".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                ),
            );
        }

        session
    }

    #[test]
    fn should_include_session_slug_header_when_provided() {
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            Some("agavra/tuicr@main/worktree"),
            &ExportDiff::default(),
        );

        assert!(
            markdown.contains("## Session: agavra/tuicr@main/worktree"),
            "expected slug header in:\n{markdown}"
        );
    }

    #[test]
    fn should_omit_session_slug_header_when_disabled_by_config() {
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;
        let export = ExportConfig {
            session_header: Some(false),
            ..ExportConfig::default()
        };

        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &export,
            &[],
            Some("agavra/tuicr@main/worktree"),
            &ExportDiff::default(),
        );

        assert!(
            !markdown.contains("## Session:"),
            "expected no slug header in:\n{markdown}"
        );
    }

    #[test]
    fn should_omit_session_slug_header_when_absent() {
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(!markdown.contains("## Session:"));
    }

    #[test]
    fn should_generate_valid_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(markdown.contains("I reviewed your code and have the following comments"));
        assert!(
            markdown.contains("Comment types: SUGGESTION (improvements), ISSUE (problems to fix)")
        );
        assert!(!markdown.contains("NOTE"));
        assert!(!markdown.contains("PRAISE"));
        assert!(markdown.contains("[SUGGESTION]"));
        assert!(markdown.contains("`src/main.rs`"));
        assert!(markdown.contains("Consider adding documentation"));
        assert!(markdown.contains("[ISSUE]"));
        assert!(markdown.contains("`src/main.rs:42`"));
        assert!(markdown.contains("Magic number"));
    }

    #[test]
    fn should_keep_commit_identity_for_commit_message_comments() {
        // Comments on the messages of different commits must not collapse into
        // an indistinguishable `Commit Message:N`. The synthetic commit-message
        // path carries the short id, which keeps each comment attributable to
        // its commit in the export.
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::CommitRange,
        );
        let first = PathBuf::from("Commit Message (ed50028)");
        let second = PathBuf::from("Commit Message (c17beb2)");
        session.add_file(first.clone(), FileStatus::Added, 0);
        session.add_file(second.clone(), FileStatus::Added, 0);
        if let Some(review) = session.get_file_mut(&first) {
            review.add_line_comment(
                1,
                Comment::new(
                    "We do not need this commit".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                ),
            );
        }
        if let Some(review) = session.get_file_mut(&second) {
            review.add_line_comment(
                6,
                Comment::new(
                    "This is wrong".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                ),
            );
        }

        let diff_source =
            DiffSource::CommitRange(vec!["ed50028".to_string(), "c17beb2".to_string()]);
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(
            markdown.contains("`Commit Message (ed50028):1`"),
            "expected first commit's message comment to be attributable in:\n{markdown}"
        );
        assert!(
            markdown.contains("`Commit Message (c17beb2):6`"),
            "expected second commit's message comment to be attributable in:\n{markdown}"
        );
    }

    #[test]
    fn should_use_configured_label_and_definition_in_export() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_file_comment(Comment::new(
                "Needs clarification".to_string(),
                CommentType::from_id("note"),
                None,
            ));
        }

        let custom_types = vec![CommentTypeDefinition {
            id: "note".to_string(),
            label: "question".to_string(),
            definition: Some("ask for clarification".to_string()),
            color: None,
        }];

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &custom_types,
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(markdown.contains("Comment types: QUESTION (ask for clarification)"));
        assert!(markdown.contains("**[QUESTION]**"));
    }

    #[test]
    fn should_number_comments_sequentially() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        // Should have 2 numbered comments
        assert!(markdown.contains("1. **[SUGGESTION]**"));
        assert!(markdown.contains("2. **[ISSUE]**"));
    }

    #[test]
    fn should_indent_multiline_comments_under_single_digit_list_marker() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_file_comment(Comment::new(
                "foo\nbar".to_string(),
                CommentType::from_id("suggestion"),
                None,
            ));
        }

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        let expected = "\
1. **[SUGGESTION]** `src/main.rs` - foo
   bar";

        assert!(
            markdown.contains(expected),
            "expected multiline comment continuation to align under list text:\n{markdown}"
        );
    }

    #[test]
    fn should_indent_multiline_comments_under_double_digit_list_marker() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            for i in 0..9 {
                review.add_file_comment(Comment::new(
                    format!("comment {i}"),
                    CommentType::from_id("note"),
                    None,
                ));
            }
            review.add_file_comment(Comment::new(
                "foo\nbar".to_string(),
                CommentType::from_id("suggestion"),
                None,
            ));
        }

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        let expected = "\
10. **[SUGGESTION]** `src/main.rs` - foo
    bar";

        assert!(
            markdown.contains(expected),
            "expected double-digit continuation to align under list text:\n{markdown}"
        );
    }

    #[test]
    fn should_include_review_comments_in_export() {
        let mut session = create_test_session();
        session.review_comments.push(Comment::new(
            "Please split this into smaller commits".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(markdown
            .contains("`Review Comment (scope: working tree changes)` - Please split this into smaller commits"));
    }

    #[test]
    fn should_include_commit_range_scope_for_review_comments() {
        let mut session = create_test_session();
        session.review_comments.push(Comment::new(
            "High-level concern across commits".to_string(),
            CommentType::from_id("issue"),
            None,
        ));

        let markdown = generate_markdown(
            &session,
            &DiffSource::CommitRange(vec!["abc1234567890".to_string()]),
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(markdown.contains(
            "`Review Comment (scope: selected commit range)` - High-level concern across commits"
        ));
    }

    #[test]
    fn should_include_commit_sha_in_export_for_commit_scoped_comments() {
        let mut session = create_test_session();
        // Add a line comment scoped to commit abc1234567890
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_line_comment(
                10,
                Comment::new(
                    "Wrong variable name".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                )
                .with_commit_id("abc1234567890"),
            );
        }

        let markdown = generate_markdown(
            &session,
            &DiffSource::CommitRange(vec!["abc1234567890".to_string()]),
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(
            markdown.contains("`src/main.rs:10` (commit abc1234) - Wrong variable name"),
            "commit-scoped comment must include the short SHA in the export; got:\n{markdown}"
        );
    }

    #[test]
    fn should_omit_commit_suffix_for_unscoped_comments() {
        let session = create_test_session();

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // Existing comments have commit_id = None — no (commit ...) suffix
        assert!(
            !markdown.contains("(commit "),
            "unscoped comments must not have a commit suffix; got:\n{markdown}"
        );
    }

    #[test]
    fn should_fail_export_when_no_comments() {
        // given
        let session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = export_to_clipboard(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TuicrError::NoComments));
    }

    #[test]
    fn should_generate_export_content_with_comments() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = generate_export_content(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("I reviewed your code"));
        assert!(content.contains("[SUGGESTION]"));
        assert!(content.contains("[ISSUE]"));
    }

    #[test]
    fn should_fail_generate_export_content_when_no_comments() {
        // given
        let session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = generate_export_content(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TuicrError::NoComments));
    }

    #[test]
    fn should_include_commit_range_in_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::CommitRange(vec![
            "abc1234567890".to_string(),
            "def4567890123".to_string(),
        ]);

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(markdown.contains("Reviewing commits: abc1234, def4567"));
    }

    #[test]
    fn should_include_single_commit_in_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::CommitRange(vec!["abc1234567890".to_string()]);

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(markdown.contains("Reviewing commit: abc1234"));
    }

    #[test]
    fn should_write_osc52_escape_sequence() {
        // given
        let text = "Hello, World!";
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, text).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        // OSC 52 format: ESC ] 52 ; c ; <base64> BEL
        assert!(output.starts_with("\x1b]52;c;"));
        assert!(output.ends_with("\x07"));
        // Verify the base64 content
        let base64_content = &output[7..output.len() - 1];
        assert_eq!(BASE64.encode(text), base64_content);
    }

    #[test]
    fn should_encode_empty_string_in_osc52() {
        // given
        let text = "";
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, text).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "\x1b]52;c;\x07");
    }

    #[test]
    fn should_encode_unicode_in_osc52() {
        // given
        let text = "こんにちは 🦀";
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, text).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        let base64_content = &output[7..output.len() - 1];
        // Decode and verify it matches original
        let decoded = String::from_utf8(BASE64.decode(base64_content).unwrap()).unwrap();
        assert_eq!(decoded, text);
    }

    #[test]
    fn should_encode_markdown_content_in_osc52() {
        // given - simulate what would be copied during export
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, &markdown).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        assert!(output.starts_with("\x1b]52;c;"));
        assert!(output.ends_with("\x07"));
        // Verify we can decode the base64 back to the original markdown
        let base64_content = &output[7..output.len() - 1];
        let decoded = String::from_utf8(BASE64.decode(base64_content).unwrap()).unwrap();
        assert_eq!(decoded, markdown);
    }

    #[test]
    fn should_export_single_line_range_as_single_line() {
        // given - a comment with a single-line range should display as L42, not L42-L42
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::single(42);
            review.add_line_comment(
                42,
                Comment::new_with_range(
                    "Single line comment".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(markdown.contains("`src/main.rs:42`"));
        assert!(!markdown.contains("`src/main.rs:42-42`"));
    }

    #[test]
    fn should_export_line_range_with_start_and_end() {
        // given - a comment spanning multiple lines
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::new(10, 15);
            review.add_line_comment(
                15, // keyed by end line
                Comment::new_with_range(
                    "Multi-line comment".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(markdown.contains("`src/main.rs:10-15`"));
        assert!(markdown.contains("Multi-line comment"));
    }

    #[test]
    fn should_export_old_side_line_range_with_tilde() {
        // given - a range comment on deleted lines (old side)
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::new(20, 25);
            review.add_line_comment(
                25, // keyed by end line
                Comment::new_with_range(
                    "Deleted lines comment".to_string(),
                    CommentType::from_id("suggestion"),
                    Some(LineSide::Old),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(markdown.contains("`src/main.rs:~20-~25`"));
    }

    #[test]
    fn should_export_single_old_side_line_with_tilde() {
        // given - a single line comment on a deleted line
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::single(30);
            review.add_line_comment(
                30,
                Comment::new_with_range(
                    "Single deleted line".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::Old),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(markdown.contains("`src/main.rs:~30`"));
        assert!(!markdown.contains("`src/main.rs:~30-~30`"));
    }

    #[test]
    fn should_handle_comment_without_line_range_field() {
        // given - backward compatibility: comment without line_range uses line number
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            // Use Comment::new which sets line_range to None
            review.add_line_comment(
                50,
                Comment::new(
                    "Old style comment".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(markdown.contains("`src/main.rs:50`"));
    }

    #[test]
    fn should_omit_legend_when_legend_is_disabled() {
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        let markdown = generate_markdown(
            &session,
            &diff_source,
            &comment_types(),
            &legend_off(),
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(!markdown.contains("Comment types:"));
        assert!(markdown.contains("[SUGGESTION]"));
        assert!(markdown.contains("[ISSUE]"));
    }

    #[test]
    fn should_only_list_used_comment_types_in_legend() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_file_comment(Comment::new(
                "Great work!".to_string(),
                CommentType::from_id("praise"),
                None,
            ));
        }

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(markdown.contains("Comment types: PRAISE (positive feedback)"));
        assert!(!markdown.contains("NOTE"));
        assert!(!markdown.contains("SUGGESTION"));
        assert!(!markdown.contains("ISSUE"));
    }

    #[test]
    fn should_export_none_typed_comments_without_marker_or_legend() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_line_comment(
                42,
                Comment::new(
                    "plain observation".to_string(),
                    CommentType::None,
                    Some(LineSide::New),
                ),
            );
        }

        // Pass the resolved default set (just `None`) to mirror an unconfigured
        // review.
        let none_only = vec![CommentTypeDefinition {
            id: CommentType::NONE_ID.to_string(),
            label: CommentType::NONE_ID.to_string(),
            definition: None,
            color: None,
        }];
        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &none_only,
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        // No `[TYPE]` marker on the comment and no legend line at all.
        assert!(
            !markdown.contains("Comment types:"),
            "unexpected legend:\n{markdown}"
        );
        assert!(
            !markdown.contains("**["),
            "unexpected type marker:\n{markdown}"
        );
        assert!(
            !markdown.contains("[NONE]"),
            "None must not render:\n{markdown}"
        );
        assert!(markdown.contains("`src/main.rs:42`"));
        assert!(markdown.contains("plain observation"));
    }

    fn sample_pr_diff_source() -> DiffSource {
        use crate::app::PullRequestDiffSource;
        use crate::forge::traits::{ForgeRepository, PrSessionKey};
        DiffSource::PullRequest(Box::new(PullRequestDiffSource {
            key: PrSessionKey::new(
                ForgeRepository::github("github.com", "agavra", "tuicr"),
                125,
                "abc1234deadbeef".to_string(),
            ),
            base_sha: "1234567890".to_string(),
            title: "Support reviews".to_string(),
            url: "https://github.com/agavra/tuicr/pull/125".to_string(),
            head_ref_name: "reviews".to_string(),
            base_ref_name: "main".to_string(),
            state: "OPEN".to_string(),
            closed: false,
            merged: false,
        }))
    }

    fn sample_remote_thread(
        id: &str,
        author: &str,
        body: &str,
        line: u32,
        resolved: bool,
    ) -> RemoteReviewThread {
        use crate::forge::remote_comments::RemoteCommentSide;
        RemoteReviewThread {
            id: id.to_string(),
            path: "src/lib.rs".to_string(),
            line: Some(line),
            side: RemoteCommentSide::Right,
            is_resolved: resolved,
            is_outdated: false,
            comments: vec![RemoteReviewComment {
                id: format!("{id}-root"),
                author: Some(author.to_string()),
                body: body.to_string(),
                url: format!("https://github.com/agavra/tuicr/pull/125#discussion_{id}"),
                ..Default::default()
            }],
        }
    }

    #[test]
    fn should_include_unresolved_remote_threads_grouped_by_file_in_pr_export() {
        // given a PR session with one local draft + one unresolved remote
        // thread + one resolved (must be omitted)
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/lib.rs")) {
            review.add_line_comment(
                10,
                Comment::new(
                    "Local draft".to_string(),
                    CommentType::from_id("issue"),
                    Some(LineSide::New),
                ),
            );
        }
        let threads = vec![
            sample_remote_thread("a", "alice", "Can this be simpler?", 42, false),
            sample_remote_thread("b", "bob", "Old resolved", 99, true),
        ];

        // when
        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &ExportConfig::default(),
            &threads,
            None,
            &ExportDiff::default(),
        );

        // then
        assert!(markdown.contains("Reviewing pull request agavra/tuicr#125"));
        assert!(markdown.contains("## Local tuicr Comments"));
        assert!(markdown.contains("[ISSUE]"));
        assert!(markdown.contains("## Existing GitHub Comments"));
        assert!(markdown.contains("### `src/lib.rs`"));
        assert!(markdown.contains("@alice - Can this be simpler?"));
        // Resolved thread is omitted from export per spec.
        assert!(
            !markdown.contains("Old resolved"),
            "resolved thread leaked into export:\n{markdown}"
        );
    }

    #[test]
    fn should_mark_unpublished_remote_drafts_in_pr_export() {
        // given a thread whose root is the viewer's own unpublished draft
        let session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        let mut thread = sample_remote_thread("a", "me", "not posted yet", 5, false);
        thread.comments[0].is_draft = true;

        // when
        let markdown = generate_markdown(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &ExportConfig::default(),
            &[thread],
            None,
            &ExportDiff::default(),
        );

        // then — a reader must not take it for a posted comment
        assert!(
            markdown.contains("@me (draft) - not posted yet"),
            "{markdown}"
        );
    }

    #[test]
    fn should_export_pr_remote_threads_even_when_no_local_drafts() {
        // given a PR session with no local comments but one unresolved remote thread
        let session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        let threads = vec![sample_remote_thread("a", "alice", "important", 5, false)];

        // when
        let result = generate_export_content(
            &session,
            &sample_pr_diff_source(),
            &comment_types(),
            &ExportConfig::default(),
            &threads,
            None,
            &ExportDiff::default(),
        );

        // then — export succeeds even with no local comments
        let content = result.unwrap();
        assert!(content.contains("## Existing GitHub Comments"));
        assert!(content.contains("@alice - important"));
    }

    #[test]
    fn should_not_include_remote_comments_outside_pr_mode_in_export() {
        // given — local working-tree mode + non-empty threads (should be ignored)
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp"),
            "abc".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, 0);
        if let Some(r) = session.get_file_mut(&PathBuf::from("src/lib.rs")) {
            r.add_line_comment(
                3,
                Comment::new(
                    "Local".to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                ),
            );
        }
        let threads = vec![sample_remote_thread("a", "alice", "ignored", 5, false)];

        // when
        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &threads,
            None,
            &ExportDiff::default(),
        );

        // then — the section is omitted in non-PR modes
        assert!(!markdown.contains("Existing GitHub Comments"));
    }

    #[test]
    fn should_only_list_used_custom_types_in_legend() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_file_comment(Comment::new(
                "Needs clarification".to_string(),
                CommentType::from_id("note"),
                None,
            ));
        }

        let custom_types = vec![
            CommentTypeDefinition {
                id: "note".to_string(),
                label: "question".to_string(),
                definition: Some("ask for clarification".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "issue".to_string(),
                label: "issue".to_string(),
                definition: Some("problems to fix".to_string()),
                color: None,
            },
        ];

        let markdown = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &custom_types,
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );

        assert!(markdown.contains("Comment types: QUESTION (ask for clarification)"));
        assert!(!markdown.contains("ISSUE"));
    }

    // code_context

    fn code_context_on() -> ExportConfig {
        ExportConfig {
            code_context: Some(true),
            ..ExportConfig::default()
        }
    }

    fn diff_line(origin: LineOrigin, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
        DiffLine {
            origin,
            content: text.to_string(),
            raw_content: None,
            old_lineno: old,
            new_lineno: new,
            highlighted_spans: None,
        }
    }

    /// `src/lib.rs` with one hunk, `@@ -10,6 +10,7 @@`.
    fn code_context_diff_file() -> DiffFile {
        use LineOrigin::{Addition, Context, Deletion};
        let lines = vec![
            diff_line(Context, Some(10), Some(10), "fn total() -> u32 {"),
            diff_line(Context, Some(11), Some(11), "    let x = 1;"),
            diff_line(Deletion, Some(12), None, "    let y = 2;"),
            diff_line(Addition, None, Some(12), "    let y = 3;"),
            diff_line(Addition, None, Some(13), "    let z = 4;"),
            diff_line(Context, Some(13), Some(14), "    x + y + z"),
            diff_line(Context, Some(14), Some(15), "}"),
            diff_line(Context, Some(15), Some(16), ""),
        ];
        let hunks = vec![DiffHunk {
            header: "@@ -10,6 +10,7 @@".to_string(),
            lines,
            old_start: 10,
            old_count: 6,
            new_start: 10,
            new_count: 7,
        }];
        DiffFile {
            old_path: Some(PathBuf::from("src/lib.rs")),
            new_path: Some(PathBuf::from("src/lib.rs")),
            status: FileStatus::Modified,
            content_hash: DiffFile::compute_content_hash(&hunks),
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
        }
    }

    /// A session whose `src/lib.rs` holds the given line comments, keyed by
    /// their range start.
    fn code_context_session(comments: Vec<Comment>) -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        let path = PathBuf::from("src/lib.rs");
        session.add_file(path.clone(), FileStatus::Modified, 0);
        let review = session.get_file_mut(&path).unwrap();
        for comment in comments {
            let line = comment.line_range.map(|r| r.start).unwrap();
            review.add_line_comment(line, comment);
        }
        session
    }

    fn line_comment(text: &str, side: LineSide, start: u32, end: u32) -> Comment {
        Comment::new_with_range(
            text.to_string(),
            CommentType::from_id("issue"),
            Some(side),
            LineRange::new(start, end),
        )
    }

    fn export_with_code(session: &ReviewSession, files: &[DiffFile]) -> String {
        generate_markdown(
            session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &code_context_on(),
            &[],
            None,
            &ExportDiff {
                files,
                subset_loaded: false,
                single_commit: None,
            },
        )
    }

    #[test]
    fn should_add_code_context_under_line_comment() {
        let session = code_context_session(vec![line_comment("Name this", LineSide::New, 13, 13)]);
        let md = export_with_code(&session, &[code_context_diff_file()]);
        assert!(md.contains(
            "1. **[ISSUE]** `src/lib.rs:13` - Name this\n\
             \x20  ```diff\n\
             \x20  @@ -12,3 +12,4 @@\n\
             \x20  -    let y = 2;\n\
             \x20  +    let y = 3;\n\
             \x20  +    let z = 4;\n\
             \x20       x + y + z\n\
             \x20   }\n\
             \x20  ```\n"
        ));
    }

    #[test]
    fn should_add_code_context_under_range_comment_clipped_at_hunk_end() {
        // Only one line follows the range inside the hunk, so the trailing
        // context stops there instead of reaching for lines the diff lacks.
        let session = code_context_session(vec![line_comment("Simplify", LineSide::New, 14, 15)]);
        let md = export_with_code(&session, &[code_context_diff_file()]);
        assert!(md.contains(
            "1. **[ISSUE]** `src/lib.rs:14-15` - Simplify\n\
             \x20  ```diff\n\
             \x20  @@ -13,3 +12,5 @@\n\
             \x20  +    let y = 3;\n\
             \x20  +    let z = 4;\n\
             \x20       x + y + z\n\
             \x20   }\n\
             \x20   \n\
             \x20  ```\n"
        ));
    }

    #[test]
    fn should_clip_code_context_at_hunk_start() {
        let session = code_context_session(vec![line_comment("Rename", LineSide::New, 10, 10)]);
        let md = export_with_code(&session, &[code_context_diff_file()]);
        assert!(md.contains(
            "   ```diff\n\
             \x20  @@ -10,3 +10,2 @@\n\
             \x20   fn total() -> u32 {\n\
             \x20       let x = 1;\n\
             \x20  -    let y = 2;\n\
             \x20  ```\n"
        ));
    }

    #[test]
    fn should_read_deleted_line_context_from_old_side() {
        let session =
            code_context_session(vec![line_comment("Why drop this?", LineSide::Old, 12, 12)]);
        let md = export_with_code(&session, &[code_context_diff_file()]);
        assert!(md.contains(
            "1. **[ISSUE]** `src/lib.rs:~12` - Why drop this?\n\
             \x20  ```diff\n\
             \x20  @@ -10,3 +10,4 @@\n\
             \x20   fn total() -> u32 {\n\
             \x20       let x = 1;\n\
             \x20  -    let y = 2;\n\
             \x20  +    let y = 3;\n\
             \x20  +    let z = 4;\n\
             \x20  ```\n"
        ));
    }

    #[test]
    fn should_not_add_code_context_to_file_or_review_comments() {
        let mut session = code_context_session(Vec::new());
        session.review_comments.push(Comment::new(
            "Overall fine".to_string(),
            CommentType::from_id("note"),
            None,
        ));
        session
            .get_file_mut(&PathBuf::from("src/lib.rs"))
            .unwrap()
            .add_file_comment(Comment::new(
                "Add docs".to_string(),
                CommentType::from_id("suggestion"),
                None,
            ));
        let md = export_with_code(&session, &[code_context_diff_file()]);
        assert!(md.contains("Add docs"));
        assert!(md.contains("Overall fine"));
        assert!(!md.contains("```"));
    }

    #[test]
    fn should_keep_export_byte_identical_when_code_context_is_off() {
        let session = code_context_session(vec![
            line_comment("Name this", LineSide::New, 13, 13),
            line_comment("Why drop this?", LineSide::Old, 12, 12),
        ]);
        let files = [code_context_diff_file()];
        let without_diff = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff::default(),
        );
        let with_diff = generate_markdown(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &ExportDiff {
                files: &files,
                subset_loaded: false,
                single_commit: None,
            },
        );
        assert_eq!(with_diff, without_diff);
        assert!(!with_diff.contains("```"));
    }

    fn export_scoped(
        session: &ReviewSession,
        subset_loaded: bool,
        single_commit: Option<&str>,
    ) -> String {
        let files = [code_context_diff_file()];
        generate_markdown(
            session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &code_context_on(),
            &[],
            None,
            &ExportDiff {
                files: &files,
                subset_loaded,
                single_commit: single_commit.map(str::to_string),
            },
        )
    }

    fn stamped(mut comment: Comment, sha: &str) -> Comment {
        comment.commit_id = Some(sha.to_string());
        comment
    }

    #[test]
    fn should_skip_code_context_for_comments_scoped_to_unloaded_commit() {
        let session = code_context_session(vec![
            stamped(
                line_comment("Shown commit", LineSide::New, 13, 13),
                "aaaaaaaaaaaa",
            ),
            stamped(
                line_comment("Other commit", LineSide::New, 10, 10),
                "bbbbbbbbbbbb",
            ),
        ]);
        let md = export_scoped(&session, true, Some("aaaaaaaaaaaa"));
        // Both comments still export; only the one whose commit is loaded
        // carries code.
        assert!(md.contains("`src/lib.rs:10` (commit bbbbbbb) - Other commit\n2. "));
        assert!(md.contains("Shown commit\n   ```diff\n   @@ -12,3 +12,4 @@\n"));
    }

    #[test]
    fn should_skip_code_context_for_unscoped_comment_over_commit_subset() {
        // Written against the full diff, so a single commit's lines at the
        // same number may be different code.
        let session = code_context_session(vec![line_comment("Full diff", LineSide::New, 13, 13)]);
        let md = export_scoped(&session, true, Some("aaaaaaaaaaaa"));
        assert!(md.contains("`src/lib.rs:13` - Full diff\n"));
        assert!(!md.contains("```"));
        // The same comment over the full diff does get its code.
        let md = export_scoped(&session, false, None);
        assert!(md.contains("Full diff\n   ```diff\n"));
    }

    #[test]
    fn should_skip_code_context_for_stamped_comment_inside_wider_subset() {
        let session = code_context_session(vec![stamped(
            line_comment("One commit", LineSide::New, 13, 13),
            "aaaaaaaaaaaa",
        )]);
        let md = export_scoped(&session, true, None);
        assert!(md.contains("(commit aaaaaaa) - One commit\n"));
        assert!(!md.contains("```"));
        // Nor over the full multi-commit diff.
        let md = export_scoped(&session, false, None);
        assert!(!md.contains("```"));
    }

    fn lib_file(hunks: Vec<DiffHunk>, status: FileStatus) -> DiffFile {
        DiffFile {
            old_path: (status != FileStatus::Added).then(|| PathBuf::from("src/lib.rs")),
            new_path: Some(PathBuf::from("src/lib.rs")),
            status,
            content_hash: DiffFile::compute_content_hash(&hunks),
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
        }
    }

    fn hunk(old: (u32, u32), new: (u32, u32), lines: Vec<DiffLine>) -> DiffHunk {
        DiffHunk {
            header: format!("@@ -{},{} +{},{} @@", old.0, old.1, new.0, new.1),
            lines,
            old_start: old.0,
            old_count: old.1,
            new_start: new.0,
            new_count: new.1,
        }
    }

    fn context_lines(range: std::ops::RangeInclusive<u32>) -> Vec<DiffLine> {
        range
            .map(|n| diff_line(LineOrigin::Context, Some(n), Some(n), &format!("line {n}")))
            .collect()
    }

    #[test]
    fn should_give_each_hunk_its_own_header_when_range_spans_hunks() {
        let file = lib_file(
            vec![
                hunk((1, 3), (1, 3), context_lines(1..=3)),
                hunk((10, 3), (10, 3), context_lines(10..=12)),
            ],
            FileStatus::Modified,
        );
        let session = code_context_session(vec![line_comment("Span", LineSide::New, 2, 11)]);
        let md = export_with_code(&session, &[file]);
        assert!(md.contains(
            "1. **[ISSUE]** `src/lib.rs:2-11` - Span\n\
             \x20  ```diff\n\
             \x20  @@ -1,3 +1,3 @@\n\
             \x20   line 1\n\
             \x20   line 2\n\
             \x20   line 3\n\
             \x20  @@ -10,3 +10,3 @@\n\
             \x20   line 10\n\
             \x20   line 11\n\
             \x20   line 12\n\
             \x20  ```\n"
        ));
    }

    #[test]
    fn should_count_both_sides_of_interleaved_slice() {
        use LineOrigin::{Addition, Context, Deletion};
        let file = lib_file(
            vec![hunk(
                (1, 7),
                (1, 8),
                vec![
                    diff_line(Context, Some(1), Some(1), "fn total() -> u32 {"),
                    diff_line(Context, Some(2), Some(2), "    let x = 1;"),
                    diff_line(Deletion, Some(3), None, "    let y = 2;"),
                    diff_line(Deletion, Some(4), None, "    x + y"),
                    diff_line(Addition, None, Some(3), "    let y = 3;"),
                    diff_line(Addition, None, Some(4), "    let z = 4;"),
                    diff_line(Addition, None, Some(5), "    x + y + z"),
                    diff_line(Context, Some(5), Some(6), "}"),
                    diff_line(Context, Some(6), Some(7), ""),
                    diff_line(Context, Some(7), Some(8), "fn other() {}"),
                ],
            )],
            FileStatus::Modified,
        );
        let session = code_context_session(vec![line_comment("Name this", LineSide::New, 4, 4)]);
        let md = export_with_code(&session, &[file]);
        assert!(md.contains(
            "   ```diff\n\
             \x20  @@ -4,2 +3,4 @@\n\
             \x20  -    x + y\n\
             \x20  +    let y = 3;\n\
             \x20  +    let z = 4;\n\
             \x20  +    x + y + z\n\
             \x20   }\n\
             \x20  ```\n"
        ));
    }

    #[test]
    fn should_name_line_zero_for_old_side_of_new_file() {
        let lines = (1..=3)
            .map(|n| diff_line(LineOrigin::Addition, None, Some(n), &format!("line {n}")))
            .collect();
        let file = lib_file(vec![hunk((0, 0), (1, 3), lines)], FileStatus::Added);
        let session = code_context_session(vec![line_comment("New", LineSide::New, 2, 2)]);
        let md = export_with_code(&session, &[file]);
        assert!(md.contains("   @@ -0,0 +1,3 @@\n   +line 1\n   +line 2\n   +line 3\n   ```\n"));
    }

    #[test]
    fn should_name_line_before_for_side_with_no_lines_in_slice() {
        // Six additions between old lines 5 and 6. A slice that holds only
        // additions names old line 5, the one before it, with a zero count.
        let mut lines = vec![diff_line(LineOrigin::Context, Some(5), Some(5), "line 5")];
        lines.extend(
            (6..=11).map(|n| diff_line(LineOrigin::Addition, None, Some(n), &format!("add {n}"))),
        );
        lines.push(diff_line(LineOrigin::Context, Some(6), Some(12), "line 6"));
        let file = lib_file(vec![hunk((5, 2), (5, 8), lines)], FileStatus::Modified);
        let session = code_context_session(vec![line_comment("Mid", LineSide::New, 8, 8)]);
        let md = export_with_code(&session, &[file]);
        assert!(md.contains(
            "   @@ -5,0 +6,5 @@\n   +add 6\n   +add 7\n   +add 8\n   +add 9\n   +add 10\n   ```\n"
        ));
    }

    #[test]
    fn should_indent_code_context_under_double_digit_list_marker() {
        let mut session =
            code_context_session(vec![line_comment("Name this", LineSide::New, 13, 13)]);
        let review = session.get_file_mut(&PathBuf::from("src/lib.rs")).unwrap();
        for n in 1..=9 {
            review.add_file_comment(Comment::new(
                format!("File note {n}"),
                CommentType::from_id("note"),
                None,
            ));
        }
        let md = export_with_code(&session, &[code_context_diff_file()]);
        assert!(md.contains(
            "10. **[ISSUE]** `src/lib.rs:13` - Name this\n    ```diff\n    @@ -12,3 +12,4 @@\n"
        ));
        assert!(md.contains("\n         x + y + z\n     }\n    ```\n"));
    }

    #[test]
    fn should_skip_code_context_when_line_is_not_in_loaded_diff() {
        let session = code_context_session(vec![line_comment("Stale", LineSide::New, 99, 99)]);
        let md = export_with_code(&session, &[code_context_diff_file()]);
        assert!(md.contains("`src/lib.rs:99` - Stale\n"));
        assert!(!md.contains("```"));
    }

    #[test]
    fn should_lengthen_fence_past_backticks_in_code() {
        let mut file = code_context_diff_file();
        file.hunks[0].lines[4].content = "    let z = \"```\";".to_string();
        let session = code_context_session(vec![line_comment("Odd", LineSide::New, 13, 13)]);
        let md = export_with_code(&session, &[file]);
        assert!(md.contains("   ````diff\n"));
        assert!(md.contains("+    let z = \"```\";\n"));
        assert!(md.contains("\n   ````\n"));
    }
}
