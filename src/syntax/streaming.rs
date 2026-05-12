//! Streaming syntax-highlight pipeline.
//!
//! The diff is parsed (and rendered) without highlighting; a background
//! worker thread then produces highlight results and streams them to the
//! main thread via an mpsc channel. The main loop drains the channel each
//! iteration and patches the model in place, then redraws.
//!
//! Why this shape: VCS access (git2, jj/hg subprocess) is often `!Send`,
//! so the parse phase has to stay on the main thread; syntect's
//! `SyntaxSet` / `Theme` are `Sync`, so the highlight phase parallelises
//! cleanly across threads as long as inputs are owned strings.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::model::diff_types::LineOrigin;
use crate::syntax::{HighlightedLines, HighlightedSpans, SyntaxHighlighter};

/// Same cost ceiling as the previous synchronous full-file pass: skip
/// highlighting for files over this size to keep runaway diffs cheap.
const MAX_HIGHLIGHT_FILE_BYTES: usize = 1024 * 1024;

/// A single unit of highlight work, produced during the (serial) parse phase
/// and consumed on the background worker.
pub(crate) struct HighlightJob {
    pub file_idx: usize,
    pub syntax_path: PathBuf,
    pub kind: HighlightJobKind,
}

pub(crate) enum HighlightJobKind {
    /// Per-hunk highlight using only the lines inside the hunk.
    Hunk {
        hunk_idx: usize,
        old_lines: Vec<String>,
        new_lines: Vec<String>,
        old_line_indices: Vec<Option<usize>>,
        new_line_indices: Vec<Option<usize>>,
        line_origins: Vec<LineOrigin>,
    },
    /// Container-grammar (Vue, Svelte, ...) full-file context. Content is
    /// fetched during parse so the worker never touches VCS state.
    FullFile {
        old_content: Option<String>,
        new_content: Option<String>,
    },
}

/// A highlight result the worker streams back. Each variant patches one
/// file in place.
pub(crate) struct HighlightUpdate {
    pub file_idx: usize,
    pub kind: HighlightUpdateKind,
}

pub(crate) enum HighlightUpdateKind {
    Hunk {
        hunk_idx: usize,
        line_spans: Vec<Option<HighlightedSpans>>,
    },
    FullFile {
        old: Option<HighlightedLines>,
        new: Option<HighlightedLines>,
    },
}

/// Shared queue of pending jobs. Workers pop from the front under the
/// mutex; the App can reorder the queue by file-index proximity when the
/// user navigates, so visible files get highlighted first.
pub(crate) type SharedQueue = Arc<Mutex<VecDeque<HighlightJob>>>;

/// Spawn the highlight worker. Returns the join handle; the caller usually
/// doesn't need to wait on it (the worker exits when its send fails on a
/// dropped receiver, or when `cancel` is set).
pub(crate) fn spawn_highlight_worker(
    queue: SharedQueue,
    highlighter: Arc<SyntaxHighlighter>,
    cancel: Arc<AtomicBool>,
    tx: Sender<HighlightUpdate>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || run_worker(queue, highlighter, cancel, tx))
}

/// Run the highlight pipeline synchronously on the current thread. Used by
/// tests that want spans populated deterministically without a worker thread.
#[cfg(test)]
pub(crate) fn run_blocking(
    jobs: Vec<HighlightJob>,
    highlighter: &SyntaxHighlighter,
) -> Vec<HighlightUpdate> {
    jobs.iter()
        .filter_map(|job| run_job(job, highlighter))
        .collect()
}

fn run_worker(
    queue: SharedQueue,
    highlighter: Arc<SyntaxHighlighter>,
    cancel: Arc<AtomicBool>,
    tx: Sender<HighlightUpdate>,
) {
    let initial_len = queue.lock().unwrap().len();
    if initial_len == 0 {
        return;
    }

    let parallelism = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(initial_len)
        .max(1);

    thread::scope(|s| {
        for _ in 0..parallelism {
            let queue = Arc::clone(&queue);
            let cancel = Arc::clone(&cancel);
            let tx = tx.clone();
            let highlighter = Arc::clone(&highlighter);
            s.spawn(move || {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let job = queue.lock().unwrap().pop_front();
                    let Some(job) = job else {
                        return;
                    };
                    if let Some(update) = run_job(&job, &highlighter)
                        && tx.send(update).is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
}

fn run_job(job: &HighlightJob, h: &SyntaxHighlighter) -> Option<HighlightUpdate> {
    match &job.kind {
        HighlightJobKind::Hunk {
            hunk_idx,
            old_lines,
            new_lines,
            old_line_indices,
            new_line_indices,
            line_origins,
        } => {
            let old_highlighted = h.highlight_file_lines(&job.syntax_path, old_lines);
            let new_highlighted = h.highlight_file_lines(&job.syntax_path, new_lines);
            if old_highlighted.is_none() && new_highlighted.is_none() {
                return None;
            }
            let line_spans: Vec<Option<HighlightedSpans>> = (0..line_origins.len())
                .map(|i| {
                    h.highlighted_line_for_diff_with_background(
                        old_highlighted.as_deref(),
                        new_highlighted.as_deref(),
                        old_line_indices[i],
                        new_line_indices[i],
                        line_origins[i],
                    )
                })
                .collect();
            Some(HighlightUpdate {
                file_idx: job.file_idx,
                kind: HighlightUpdateKind::Hunk {
                    hunk_idx: *hunk_idx,
                    line_spans,
                },
            })
        }
        HighlightJobKind::FullFile {
            old_content,
            new_content,
        } => {
            let old = highlight_full(h, &job.syntax_path, old_content.as_deref());
            let new = highlight_full(h, &job.syntax_path, new_content.as_deref());
            if old.is_none() && new.is_none() {
                return None;
            }
            Some(HighlightUpdate {
                file_idx: job.file_idx,
                kind: HighlightUpdateKind::FullFile { old, new },
            })
        }
    }
}

fn highlight_full(
    h: &SyntaxHighlighter,
    path: &std::path::Path,
    content: Option<&str>,
) -> Option<HighlightedLines> {
    let content = content?;
    if content.len() > MAX_HIGHLIGHT_FILE_BYTES || content.as_bytes().contains(&0u8) {
        return None;
    }
    let lines: Vec<String> = content.lines().map(crate::vcs::tabify).collect();
    h.highlight_file_lines(path, &lines)
}

/// A running highlight session: receiver for streamed results, the cancel
/// token shared with the worker, and the shared job queue (so the App can
/// reprioritize pending work when the user navigates).
pub(crate) struct HighlightSession {
    rx: mpsc::Receiver<HighlightUpdate>,
    cancel: Arc<AtomicBool>,
    queue: SharedQueue,
}

impl HighlightSession {
    /// Start a new session. Returns `None` if there is nothing to highlight,
    /// in which case no worker is spawned.
    pub(crate) fn start(
        jobs: Vec<HighlightJob>,
        highlighter: Arc<SyntaxHighlighter>,
    ) -> Option<Self> {
        if jobs.is_empty() {
            return None;
        }
        let queue: SharedQueue = Arc::new(Mutex::new(VecDeque::from(jobs)));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        spawn_highlight_worker(Arc::clone(&queue), highlighter, Arc::clone(&cancel), tx);
        Some(Self { rx, cancel, queue })
    }

    /// Re-resolve each job's `file_idx` by `syntax_path` against
    /// `diff_files.display_path()`, drop jobs whose path is no longer present,
    /// then start the session. Used after the parse-time `file_idx` may have
    /// drifted from its file's position (commit-message insert, directory
    /// sort, `.tuicrignore` re-filter). `HighlightJob::syntax_path` and
    /// `DiffFile::display_path` are both `new_path or old_path`, so the keys
    /// line up.
    pub(crate) fn start_resolved(
        mut jobs: Vec<HighlightJob>,
        diff_files: &[crate::model::DiffFile],
        highlighter: Arc<SyntaxHighlighter>,
    ) -> Option<Self> {
        let path_to_idx: HashMap<&Path, usize> = diff_files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.display_path().as_path(), i))
            .collect();
        jobs.retain_mut(|job| match path_to_idx.get(job.syntax_path.as_path()) {
            Some(&idx) => {
                job.file_idx = idx;
                true
            }
            None => false,
        });
        Self::start(jobs, highlighter)
    }

    /// Signal the worker to stop. The receiver is also dropped, so any
    /// in-flight result the worker tries to send will fail and the worker
    /// exits at its next channel send.
    pub(crate) fn cancel(self) {
        self.cancel.store(true, Ordering::Relaxed);
        // drop self → rx dropped
    }

    pub(crate) fn try_recv(&self) -> Result<HighlightUpdate, mpsc::TryRecvError> {
        self.rx.try_recv()
    }

    /// Reorder the pending queue so jobs nearest `file_idx` run next.
    /// Called when the user navigates to a new file, so the visible
    /// viewport gets highlighted before far-away files.
    pub(crate) fn prioritize_around(&self, file_idx: usize) {
        let mut q = self.queue.lock().unwrap();
        q.make_contiguous()
            .sort_by_key(|job| job.file_idx.abs_diff(file_idx));
    }
}

/// Patch a streamed highlight result into the in-memory diff. Idempotent:
/// applying the same update twice produces the same final state.
pub(crate) fn apply_update(
    files: &mut [crate::model::DiffFile],
    highlighter: &SyntaxHighlighter,
    update: HighlightUpdate,
) {
    let Some(file) = files.get_mut(update.file_idx) else {
        return;
    };
    match update.kind {
        HighlightUpdateKind::Hunk {
            hunk_idx,
            line_spans,
        } => {
            let Some(hunk) = file.hunks.get_mut(hunk_idx) else {
                return;
            };
            for (line, spans) in hunk.lines.iter_mut().zip(line_spans) {
                if let Some(spans) = spans {
                    line.highlighted_spans = Some(spans);
                }
            }
        }
        HighlightUpdateKind::FullFile { old, new } => {
            for hunk in &mut file.hunks {
                for line in &mut hunk.lines {
                    let old_idx = line.old_lineno.map(|n| n.saturating_sub(1) as usize);
                    let new_idx = line.new_lineno.map(|n| n.saturating_sub(1) as usize);
                    let spans = highlighter.highlighted_line_for_diff_with_background(
                        old.as_deref(),
                        new.as_deref(),
                        old_idx,
                        new_idx,
                        line.origin,
                    );
                    if spans.is_some() {
                        line.highlighted_spans = spans;
                    }
                }
            }
        }
    }
}
