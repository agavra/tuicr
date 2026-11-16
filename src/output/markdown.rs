use std::collections::HashSet;
use std::fmt::Write;
use std::io::Write as IoWrite;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::app::{CommentTypeDefinition, DiffSource, resolve_comment_type_label};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CopyMethod {
    // macOS system pasteboard
    Pbcopy,
    // xclip / wl-copy — direct Wayland/X11 clipboard
    SubProcess,
    // library clipboard
    Arboard,
    // tmux load-buffer -w / OSC 52
    TerminalRelay,
}

impl CopyMethod {
    pub(crate) fn from_config_name(name: &str) -> Option<Self> {
        match name {
            "pbcopy" => Some(Self::Pbcopy),
            "subprocess" => Some(Self::SubProcess),
            "arboard" => Some(Self::Arboard),
            "osc52" => Some(Self::TerminalRelay),
            _ => None,
        }
    }

    pub(crate) fn config_name(self) -> &'static str {
        match self {
            Self::Pbcopy => "pbcopy",
            Self::SubProcess => "subprocess",
            Self::Arboard => "arboard",
            Self::TerminalRelay => "osc52",
        }
    }
}

pub(crate) const CLIPBOARD_MECHANISMS: &[&str] = &["arboard", "pbcopy", "subprocess", "osc52"];

/// Whether a forced mechanism cannot work on the given platform. Used to warn
/// (not block) at startup; the `TerminalRelay` backstop still guarantees a copy.
pub(crate) fn mechanism_unsupported(method: CopyMethod, is_macos: bool) -> bool {
    match method {
        CopyMethod::Pbcopy => !is_macos,
        CopyMethod::SubProcess => is_macos,
        CopyMethod::Arboard | CopyMethod::TerminalRelay => false,
    }
}

pub(crate) fn resolve_clipboard_override(names: Option<&[String]>) -> Option<Vec<CopyMethod>> {
    let methods: Vec<CopyMethod> = names?
        .iter()
        .filter_map(|name| CopyMethod::from_config_name(name))
        .collect();
    if methods.is_empty() {
        None
    } else {
        Some(methods)
    }
}

/// Environment inputs that decide clipboard strategy.
struct ClipboardEnv {
    is_macos: bool,
    ssh: bool,
    tmux: bool,
    zellij: bool,
    rex: bool,
}

impl ClipboardEnv {
    fn detect() -> Self {
        Self {
            is_macos: cfg!(target_os = "macos"),
            ssh: std::env::var_os("SSH_TTY").is_some()
                || std::env::var_os("SSH_CONNECTION").is_some(),
            tmux: std::env::var_os("TMUX").is_some(),
            zellij: std::env::var_os("ZELLIJ").is_some(),
            rex: std::env::var_os("REX_SESSION").is_some()
                || std::env::var("TERM_PROGRAM").is_ok_and(|program| program == "rex"),
        }
    }
}

fn copy_method_order(env: &ClipboardEnv, override_order: Option<&[CopyMethod]>) -> Vec<CopyMethod> {
    if let Some(methods) = override_order {
        return with_terminal_backstop(methods);
    }
    if env.is_macos {
        return vec![
            CopyMethod::Pbcopy,
            CopyMethod::Arboard,
            CopyMethod::TerminalRelay,
        ];
    }
    // Local tools would copy on the remote host, not the user's machine.
    if env.ssh || env.rex {
        return vec![CopyMethod::TerminalRelay];
    }
    if env.tmux || env.zellij {
        return vec![CopyMethod::SubProcess, CopyMethod::TerminalRelay];
    }
    vec![
        CopyMethod::SubProcess,
        CopyMethod::Arboard,
        CopyMethod::TerminalRelay,
    ]
}

/// Append the terminal fallback when the configured order omits it.
fn with_terminal_backstop(methods: &[CopyMethod]) -> Vec<CopyMethod> {
    let mut order = methods.to_vec();
    if !order.contains(&CopyMethod::TerminalRelay) {
        order.push(CopyMethod::TerminalRelay);
    }
    order
}

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
    /// The loaded single commit; `None` for cumulative or working-tree views.
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
    export_to_clipboard_with(
        session,
        diff_source,
        comment_types,
        export,
        remote_threads,
        session_slug,
        diff,
        None,
        None,
    )
    .map(|copy| copy.message("Review"))
}

/// Export using the configured command and copy order.
#[allow(clippy::too_many_arguments)]
pub(crate) fn export_to_clipboard_with(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
    export: &ExportConfig,
    remote_threads: &[RemoteReviewThread],
    session_slug: Option<&str>,
    diff: &ExportDiff,
    clipboard_override: Option<&[CopyMethod]>,
    clipboard_command: Option<&str>,
) -> Result<ClipboardCopy> {
    let content = generate_export_content(
        session,
        diff_source,
        comment_types,
        export,
        remote_threads,
        session_slug,
        diff,
    )?;
    copy_text_to_clipboard_with(&content, clipboard_override, clipboard_command)
}

/// Write the same review `:clip` copies to `path`, replacing any file there.
pub fn export_to_file(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
    export: &ExportConfig,
    remote_threads: &[RemoteReviewThread],
    session_slug: Option<&str>,
    path: &std::path::Path,
) -> Result<String> {
    let content = generate_export_content(
        session,
        diff_source,
        comment_types,
        export,
        remote_threads,
        session_slug,
    )?;
    std::fs::write(path, content)?;
    Ok(format!("Review written to {}", path.display()))
}

/// Copy arbitrary text to the system clipboard. Returns `Ok(true)` if the
/// terminal relay (tmux / OSC 52) was used, `Ok(false)` if a direct system
/// clipboard handled it.
pub fn copy_text_to_clipboard(text: &str) -> Result<bool> {
    copy_text_to_clipboard_with(text, None, None).map(|copy| copy.via_terminal)
}

/// Copy using `clipboard_command`, then the configured order (automatic
/// detection for `None`).
pub(crate) fn copy_text_to_clipboard_with(
    text: &str,
    clipboard_override: Option<&[CopyMethod]>,
    clipboard_command: Option<&str>,
) -> Result<ClipboardCopy> {
    copy_with_command(text, clipboard_command, CLIPBOARD_COMMAND_TIMEOUT, |text| {
        copy_with_method_order(text, clipboard_override)
    })
}

/// How long `clipboard_command` may run before it is killed. It bounds how
/// long a stuck command freezes the TUI.
pub(crate) const CLIPBOARD_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

const VIA_TMUX: &str = "tmux load-buffer";
const VIA_OSC52: &str = "OSC 52";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClipboardCopy {
    /// The terminal relay (tmux / OSC 52) copied the text.
    pub(crate) via_terminal: bool,
    /// Set when `clipboard_command` failed: the failure and the method that
    /// copied instead.
    pub(crate) fallback_warning: Option<String>,
}

impl ClipboardCopy {
    pub(crate) fn message(&self, what: &str) -> String {
        if self.via_terminal {
            format!("{what} copied to clipboard (via terminal)")
        } else {
            format!("{what} copied to clipboard")
        }
    }
}

/// Try `command`, then `fallback` (which returns the method that copied).
fn copy_with_command(
    text: &str,
    command: Option<&str>,
    timeout: Duration,
    fallback: impl FnOnce(&str) -> Result<&'static str>,
) -> Result<ClipboardCopy> {
    let failure = match command.map(|command| run_clipboard_command(command, text, timeout)) {
        Some(Ok(())) => {
            return Ok(ClipboardCopy {
                via_terminal: false,
                fallback_warning: None,
            });
        }
        Some(Err(failure)) => Some(failure),
        None => None,
    };
    let via = fallback(text).map_err(|e| match &failure {
        Some(failure) => TuicrError::Clipboard(format!("{failure}; fallback failed: {e}")),
        None => e,
    })?;
    Ok(ClipboardCopy {
        via_terminal: via == VIA_TMUX || via == VIA_OSC52,
        fallback_warning: failure.map(|failure| format!("{failure}; copied via {via}")),
    })
}

fn copy_with_method_order(
    text: &str,
    clipboard_override: Option<&[CopyMethod]>,
) -> Result<&'static str> {
    for method in copy_method_order(&ClipboardEnv::detect(), clipboard_override) {
        match method {
            CopyMethod::Pbcopy if try_clipboard_cmd("pbcopy", &[], text) => return Ok("pbcopy"),
            CopyMethod::SubProcess => {
                if let Some(tool) = try_copy_via_subprocess(text) {
                    return Ok(tool);
                }
            }
            CopyMethod::Arboard
                if Clipboard::new()
                    .and_then(|mut cb| cb.set_text(text))
                    .is_ok() =>
            {
                return Ok("arboard");
            }
            CopyMethod::TerminalRelay => return copy_osc52(text),
            _ => {}
        }
    }
    // Breaks loudly if a future branch forgets to add it
    unreachable!("copy_method_order must end with TerminalRelay")
}

/// Run the user's `clipboard_command` through the shell with the text on
/// stdin. Success needs the whole text written and exit 0 within `timeout`.
fn run_clipboard_command(
    command: &str,
    text: &str,
    timeout: Duration,
) -> std::result::Result<(), String> {
    use std::process::{Command, Stdio};
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let mut cmd = Command::new(shell);
    cmd.args([flag, command])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Own process group, so a timeout also kills what the shell started.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("clipboard_command could not start ({e})"))?;
    let deadline = Instant::now() + timeout;
    let timed_out = || format!("clipboard_command timed out after {timeout:?}");

    // Written from a thread so a command that never reads cannot outlast the
    // timeout. A stdin holder that escapes the group (Windows, setsid) leaves
    // this detached thread blocked until it exits: at most one per copy.
    let (written_tx, written_rx) = std::sync::mpsc::channel();
    let stdin = child.stdin.take();
    let input = text.to_owned();
    std::thread::spawn(move || {
        let written = stdin.is_some_and(|mut stdin| stdin.write_all(input.as_bytes()).is_ok());
        let _ = written_tx.send(written);
    });

    let mut poll = Duration::from_millis(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(poll);
                poll = (poll * 2).min(Duration::from_millis(20));
            }
            waited => {
                kill_and_reap(&mut child);
                return Err(match waited {
                    Err(e) => format!("clipboard_command failed ({e})"),
                    _ => timed_out(),
                });
            }
        }
    };
    if !status.success() {
        return Err(format!("clipboard_command failed ({status})"));
    }
    // The child is reaped and its group id may be reused, so signal nothing.
    let remaining = deadline.saturating_duration_since(Instant::now());
    match written_rx.recv_timeout(remaining.max(Duration::from_millis(50))) {
        Ok(true) => Ok(()),
        Ok(false) => Err("clipboard_command did not read all copied text".to_string()),
        Err(_) => Err(timed_out()),
    }
}

/// Kill the unreaped `child` and its process group, then reap it.
fn kill_and_reap(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(pgid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: plain syscall; the child is not reaped yet, so the group
        // `process_group(0)` created still belongs to it.
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Try xclip (X11) then wl-copy (Wayland). Returns the tool that succeeded.
fn try_copy_via_subprocess(text: &str) -> Option<&'static str> {
    let session = std::env::var("XDG_SESSION_TYPE");
    if session.is_err() {
        // Session not specified
        return None;
    }
    let session = session.unwrap();
    if session == "wayland" {
        try_clipboard_cmd("wl-copy", &[], text).then_some("wl-copy")
    } else if session == "x11" {
        try_clipboard_cmd("xclip", &["-selection", "clipboard"], text).then_some("xclip")
    } else {
        // Session type unsupported
        None
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
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    matches!(child.wait(), Ok(s) if s.success()) && written
}

/// Copy text to clipboard using OSC 52 escape sequence.
/// In tmux, raw OSC 52 is intercepted and may not reach the outer terminal.
/// We use `tmux load-buffer -w` which tells tmux to handle the clipboard copy itself.
fn copy_osc52(text: &str) -> Result<&'static str> {
    if std::env::var("TMUX").is_ok() {
        copy_via_tmux(text).map(|()| VIA_TMUX)
    } else {
        let mut stdout = std::io::stdout().lock();
        write_osc52(&mut stdout, text).map(|()| VIA_OSC52)
    }
}

/// Copy text to the system clipboard via `tmux load-buffer -w -`.
///
/// The `-w` flag stores the text in tmux's buffer and forwards it to the outer
/// terminal's clipboard via OSC 52. If any of the following is missing, the copy
/// only reaches tmux's buffer — the usual cause of "clipboard empty inside tmux":
///
/// 1. `set -g set-clipboard on` in `~/.tmux.conf`. `external` and `off` both
///    stop the outward OSC 52 and the copy fails.
/// 2. tmux must know the terminal supports the `Ms` clipboard capability. In
///    tmux 3.2+, add `set -as terminal-features ',*:clipboard'`. Without it,
///    tmux drops the passthrough even with `set-clipboard on`.
/// 3. The outer terminal must allow OSC 52 writes. On by default in kitty,
///    WezTerm, iTerm2, and foot; off or opt-in in many VTE terminals and xterm.
///
/// Some terminals cap OSC 52 payload size, so large reviews can truncate silently.
///
/// On a local Linux desktop this relay is unnecessary: xclip/wl-copy write to
/// the display server directly. See `copy_method_order`; the relay is the
/// fallback and the preferred path only for SSH.
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

/// Write the OSC 52 clipboard escape sequence to `writer`.
///
/// The direct (non-tmux) relay: the escape travels the terminal stream and the
/// outer terminal sets the clipboard. Only works when that terminal allows OSC 52
/// writes (see `copy_via_tmux` for which terminals do). Separated from I/O for
/// testability.
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
            resolve_comment_type_label(&comment.comment_type, comment_types),
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
                resolve_comment_type_label(&comment.comment_type, comment_types),
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
                    resolve_comment_type_label(&comment.comment_type, comment_types),
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
                        let marker = format!("{thread_n}.");
                        let indent = " ".repeat(marker.len() + 1);
                        write_remote_body(
                            &mut md,
                            &format!("{marker} `{path}{line_marker}` @{author}{draft} - "),
                            &indent,
                            &root.body,
                        );
                        if !root.url.is_empty() {
                            let _ = writeln!(md, "{indent}<{}>", root.url);
                        }
                        for reply in thread.replies() {
                            let reply_author = reply.author.as_deref().unwrap_or("unknown");
                            write_remote_body(
                                &mut md,
                                &format!("{indent}- @{reply_author}{} - ", draft_marker(reply)),
                                &format!("{indent}  "),
                                &reply.body,
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

/// Write a remote body after `prefix`, stripping `\r` and indenting every
/// continuation line so the body stays inside its list item.
fn write_remote_body(md: &mut String, prefix: &str, indent: &str, body: &str) {
    // CRLF and lone CR are line breaks to CommonMark, so normalize both.
    let body = body.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = body.split('\n');
    let _ = writeln!(md, "{prefix}{}", lines.next().unwrap_or_default());
    for line in lines {
        let _ = writeln!(md, "{indent}{line}");
    }
}

/// Quote code only for the comment's exact diff scope. Visibility alone allows
/// legacy comments over subsets and stamped comments over wider diffs.
fn comment_addresses_loaded_diff(comment: &Comment, diff: &ExportDiff) -> bool {
    match &comment.commit_id {
        None => !diff.subset_loaded,
        Some(sha) => diff.single_commit.as_deref() == Some(sha.as_str()),
    }
}

/// Quote the range plus [`CODE_CONTEXT_LINES`] rows either side, clipped to
/// each hunk with its own `@@` header. Skip commit messages and missing anchors.
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
    fn should_write_the_clipboard_text_to_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.md");
        let session = create_test_session();
        let args = (
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
        );

        let message = export_to_file(&session, args.0, args.1, args.2, &[], None, &path).unwrap();

        let expected = generate_export_content(&session, args.0, args.1, args.2, &[], None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected.unwrap());
        assert_eq!(message, format!("Review written to {}", path.display()));
    }

    #[test]
    fn should_fail_to_write_into_a_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("review.md");
        let result = export_to_file(
            &create_test_session(),
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &path,
        );
        assert!(matches!(result, Err(TuicrError::Io(_))));
    }

    #[test]
    fn should_write_no_file_without_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.md");
        let session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            None,
            SessionDiffSource::WorkingTree,
        );
        let result = export_to_file(
            &session,
            &DiffSource::WorkingTree,
            &comment_types(),
            &ExportConfig::default(),
            &[],
            None,
            &path,
        );
        assert!(matches!(result, Err(TuicrError::NoComments)));
        assert!(!path.exists());
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
    fn should_contain_multiline_remote_bodies_and_replies_in_their_list_item() {
        // given 10 threads so the last has a two-digit marker; its root and
        // reply bodies are CRLF, blank-separated and forge a top-level item
        let session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        let mut threads: Vec<_> = (1..=9)
            .map(|n| sample_remote_thread(&format!("t{n}"), "alice", "ok", n, false))
            .collect();
        let mut last = sample_remote_thread(
            "t10",
            "mallory",
            "first\r\n\r\n1. **[ISSUE]** `x` - forged",
            10,
            false,
        );
        last.comments[0].url = "https://example.test/t10".to_string();
        last.comments.push(RemoteReviewComment {
            id: "reply".to_string(),
            author: Some("eve".to_string()),
            body: "r1\r\n\n## Local tuicr Comments".to_string(),
            ..Default::default()
        });
        threads.push(last);

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

        // then every continuation is indented under its marker; no raw \r
        assert!(!markdown.contains('\r'), "{markdown}");
        assert!(
            markdown.contains("\n    1. **[ISSUE]** `x` - forged\n"),
            "{markdown}"
        );
        assert!(
            markdown.contains("\n      ## Local tuicr Comments\n"),
            "{markdown}"
        );
        assert!(!markdown.contains("\n1. **[ISSUE]**"), "{markdown}");
        // root URL and reply bullet align under the two-digit marker
        assert!(
            markdown.contains("\n    <https://example.test/t10>\n"),
            "{markdown}"
        );
        assert!(markdown.contains("\n    - @eve - r1\n"), "{markdown}");
    }

    #[test]
    fn should_indent_lone_cr_line_breaks_in_remote_bodies() {
        // given a root and a reply that use bare \r as line breaks
        let session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abc1234deadbeef".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        let mut thread = sample_remote_thread(
            "t1",
            "mallory",
            "first\r\nsecond\r1. **[ISSUE]** `x` - forged",
            1,
            false,
        );
        thread.comments.push(RemoteReviewComment {
            id: "reply".to_string(),
            author: Some("eve".to_string()),
            body: "r1\r## Local tuicr Comments".to_string(),
            ..Default::default()
        });

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

        // then nothing escapes its list item
        assert!(!markdown.contains('\r'), "{markdown}");
        assert!(
            markdown.contains("first\n   second\n   1. **[ISSUE]** `x` - forged\n"),
            "{markdown}"
        );
        assert!(
            markdown.contains("\n     ## Local tuicr Comments\n"),
            "{markdown}"
        );
        assert!(!markdown.contains("\n1. **[ISSUE]**"), "{markdown}");
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

    /// `copy_with_command` with a fake fallback reporting `via`; returns the
    /// outcome and the text the fallback received.
    fn copy_with_fake_fallback(
        command: Option<&str>,
        text: &str,
        timeout: Duration,
        via: &'static str,
    ) -> (ClipboardCopy, Option<String>) {
        let mut received = None;
        let copy = copy_with_command(text, command, timeout, |text| {
            received = Some(text.to_string());
            Ok(via)
        })
        .unwrap();
        (copy, received)
    }

    #[cfg(unix)]
    const TEST_TIMEOUT: Duration = Duration::from_secs(2);

    /// Waits for the pid `command` wrote, once the copy has returned.
    #[cfg(unix)]
    fn read_pid(path: &std::path::Path) -> libc::pid_t {
        std::fs::read_to_string(path)
            .expect("command wrote no pid before the timeout")
            .trim()
            .parse()
            .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_command_receives_text_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.txt");
        let command = format!("cat > '{}'", out.display());
        let (copy, fallback) =
            copy_with_fake_fallback(Some(&command), "hello\nworld", TEST_TIMEOUT, "xclip");
        assert_eq!(copy.fallback_warning, None);
        assert_eq!(fallback, None, "fallback ran after the command succeeded");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "hello\nworld");
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_command_timeout_kills_reaps_and_falls_back_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let command = format!("echo $$ > '{}'; exec sleep 60", pid_file.display());
        let started = Instant::now();
        let (copy, fallback) =
            copy_with_fake_fallback(Some(&command), "hello", TEST_TIMEOUT, "xclip");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(
            copy.fallback_warning.as_deref(),
            Some("clipboard_command timed out after 2s; copied via xclip")
        );
        assert_eq!(fallback.as_deref(), Some("hello"));
        let pid = read_pid(&pid_file);
        // SAFETY: signal 0 only probes the pid; a zombie would still succeed.
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "{pid} not killed and reaped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_command_timeout_holds_when_command_never_reads_stdin() {
        // More than a pipe buffer, so the write itself blocks.
        let text = "x".repeat(4 * 1024 * 1024);
        let started = Instant::now();
        let (copy, _) = copy_with_fake_fallback(Some("sleep 60"), &text, TEST_TIMEOUT, VIA_OSC52);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(copy.via_terminal);
        assert_eq!(
            copy.fallback_warning.as_deref(),
            Some("clipboard_command timed out after 2s; copied via OSC 52")
        );
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_command_timeout_kills_processes_the_shell_started() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let command = format!("sleep 60 & echo $! > '{}'; wait", pid_file.display());
        let (copy, _) = copy_with_fake_fallback(Some(&command), "hello", TEST_TIMEOUT, "arboard");
        assert!(copy.fallback_warning.is_some());
        let pid = read_pid(&pid_file).to_string();
        // init reaps the orphan, so poll until it is gone or a zombie.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let ps = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid])
                .output()
                .unwrap();
            let stat = String::from_utf8_lossy(&ps.stdout);
            if stat.trim().is_empty() || stat.trim().starts_with('Z') {
                break;
            }
            assert!(Instant::now() < deadline, "{pid} survived the timeout");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_command_partial_stdin_falls_back_with_warning() {
        let text = "x".repeat(1_000_000);
        let (copy, fallback) =
            copy_with_fake_fallback(Some("head -c 1 >/dev/null"), &text, TEST_TIMEOUT, VIA_TMUX);
        assert!(copy.via_terminal);
        assert_eq!(
            copy.fallback_warning.as_deref(),
            Some("clipboard_command did not read all copied text; copied via tmux load-buffer")
        );
        assert_eq!(fallback, Some(text));
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_command_nonzero_exit_falls_back_with_warning() {
        for (command, status) in [("exit 3", 3), ("no-such-tuicr-command", 127)] {
            let (copy, fallback) =
                copy_with_fake_fallback(Some(command), "hello", TEST_TIMEOUT, "arboard");
            assert_eq!(
                copy.fallback_warning,
                Some(format!(
                    "clipboard_command failed (exit status: {status}); copied via arboard"
                ))
            );
            assert_eq!(fallback.as_deref(), Some("hello"));
        }
    }

    #[test]
    fn clipboard_without_command_uses_fallback_without_warning() {
        for (via, via_terminal) in [("xclip", false), (VIA_TMUX, true)] {
            let (copy, fallback) =
                copy_with_fake_fallback(None, "hello", CLIPBOARD_COMMAND_TIMEOUT, via);
            assert_eq!(
                copy,
                ClipboardCopy {
                    via_terminal,
                    fallback_warning: None,
                }
            );
            assert_eq!(fallback.as_deref(), Some("hello"));
        }
    }

    #[test]
    fn should_map_config_name_to_copy_method() {
        assert_eq!(
            CopyMethod::from_config_name("arboard"),
            Some(CopyMethod::Arboard)
        );
        assert_eq!(
            CopyMethod::from_config_name("pbcopy"),
            Some(CopyMethod::Pbcopy)
        );
        assert_eq!(
            CopyMethod::from_config_name("subprocess"),
            Some(CopyMethod::SubProcess)
        );
        assert_eq!(
            CopyMethod::from_config_name("osc52"),
            Some(CopyMethod::TerminalRelay)
        );
        assert_eq!(CopyMethod::from_config_name("nope"), None);
    }

    fn env(is_macos: bool, ssh: bool, tmux: bool, zellij: bool) -> ClipboardEnv {
        ClipboardEnv {
            is_macos,
            ssh,
            tmux,
            zellij,
            rex: false,
        }
    }

    #[test]
    fn should_keep_auto_order_when_override_is_none() {
        // Automatic order covers each environment, including remote-session precedence.
        let cases = [
            (
                env(true, false, false, false),
                vec![
                    CopyMethod::Pbcopy,
                    CopyMethod::Arboard,
                    CopyMethod::TerminalRelay,
                ],
            ),
            (
                env(false, true, false, false),
                vec![CopyMethod::TerminalRelay],
            ),
            (
                env(false, false, true, false),
                vec![CopyMethod::SubProcess, CopyMethod::TerminalRelay],
            ),
            (
                env(false, false, false, true),
                vec![CopyMethod::SubProcess, CopyMethod::TerminalRelay],
            ),
            (
                ClipboardEnv {
                    rex: true,
                    ..env(false, false, true, false)
                },
                vec![CopyMethod::TerminalRelay],
            ),
            (
                env(true, true, true, false),
                vec![
                    CopyMethod::Pbcopy,
                    CopyMethod::Arboard,
                    CopyMethod::TerminalRelay,
                ],
            ),
            (
                env(false, true, true, false),
                vec![CopyMethod::TerminalRelay],
            ),
            (
                env(false, false, false, false),
                vec![
                    CopyMethod::SubProcess,
                    CopyMethod::Arboard,
                    CopyMethod::TerminalRelay,
                ],
            ),
        ];
        for (e, want) in cases {
            assert_eq!(copy_method_order(&e, None), want);
        }
    }

    #[test]
    fn should_prioritize_override_over_environment() {
        let order = copy_method_order(&env(true, false, true, false), Some(&[CopyMethod::Arboard]));
        assert_eq!(order[0], CopyMethod::Arboard);
    }

    #[test]
    fn should_append_terminal_relay_backstop_when_override_omits_it() {
        let order = copy_method_order(
            &env(false, false, false, false),
            Some(&[CopyMethod::Arboard]),
        );
        assert_eq!(order, vec![CopyMethod::Arboard, CopyMethod::TerminalRelay]);
        assert_eq!(*order.last().unwrap(), CopyMethod::TerminalRelay);
    }

    #[test]
    fn should_not_duplicate_terminal_relay_when_override_includes_it() {
        let order = copy_method_order(
            &env(false, false, false, false),
            Some(&[CopyMethod::SubProcess, CopyMethod::TerminalRelay]),
        );
        assert_eq!(
            order,
            vec![CopyMethod::SubProcess, CopyMethod::TerminalRelay]
        );
        assert_eq!(
            order
                .iter()
                .filter(|m| **m == CopyMethod::TerminalRelay)
                .count(),
            1
        );
    }

    #[test]
    fn should_honor_single_terminal_relay_override() {
        let order = copy_method_order(
            &env(false, false, false, false),
            Some(&[CopyMethod::TerminalRelay]),
        );
        assert_eq!(order, vec![CopyMethod::TerminalRelay]);
    }

    #[test]
    fn should_resolve_config_names_to_ordered_methods() {
        let names = vec!["arboard".to_string(), "osc52".to_string()];
        assert_eq!(
            resolve_clipboard_override(Some(&names)),
            Some(vec![CopyMethod::Arboard, CopyMethod::TerminalRelay])
        );
        assert_eq!(resolve_clipboard_override(None), None);
        let empty: Vec<String> = Vec::new();
        assert_eq!(resolve_clipboard_override(Some(&empty)), None);
    }

    #[test]
    fn should_flag_platform_unsupported_mechanisms() {
        assert!(mechanism_unsupported(CopyMethod::SubProcess, true));
        assert!(!mechanism_unsupported(CopyMethod::Pbcopy, true));
        assert!(mechanism_unsupported(CopyMethod::Pbcopy, false));
        assert!(!mechanism_unsupported(CopyMethod::SubProcess, false));
        for is_macos in [true, false] {
            assert!(!mechanism_unsupported(CopyMethod::Arboard, is_macos));
            assert!(!mechanism_unsupported(CopyMethod::TerminalRelay, is_macos));
        }
    }

    #[test]
    fn config_mechanisms_match_mapping_domain() {
        use std::collections::HashSet;
        for name in CLIPBOARD_MECHANISMS {
            let method = CopyMethod::from_config_name(name)
                .unwrap_or_else(|| panic!("config mechanism {name} has no CopyMethod mapping"));
            assert_eq!(
                method.config_name(),
                *name,
                "round-trip mismatch for {name}"
            );
        }
        // Distinct methods, so validation and translation cannot silently diverge.
        let mapped: HashSet<CopyMethod> = CLIPBOARD_MECHANISMS
            .iter()
            .filter_map(|n| CopyMethod::from_config_name(n))
            .collect();
        assert_eq!(
            mapped.len(),
            CLIPBOARD_MECHANISMS.len(),
            "every config mechanism must map to a distinct CopyMethod"
        );
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
