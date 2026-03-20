use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use sha2::{Digest, Sha256};

use super::protocol::{self, InboundMessage, OutboundMessage, EventPayload};

/// Events sent from the socket server thread to the main TUI thread.
#[derive(Debug)]
pub enum McpChannelEvent {
    Connected,
    Disconnected,
    /// Server could not bind (another instance is running for this repo).
    BindFailed(String),
}

/// Review status snapshot, populated by the main thread for the socket server to read.
#[derive(Debug, Clone)]
pub struct ReviewStatus {
    pub summary: String,
    pub comment_count: usize,
    pub files_reviewed: usize,
    pub files_total: usize,
}

/// Shared state between the main TUI thread and the socket server thread.
pub struct McpChannelState {
    /// Exported feedback markdown, written by main thread, read/cleared by server.
    pub feedback: Mutex<Option<String>>,
    /// Condvar to wake blocking `poll_feedback(wait=true)` requests.
    pub feedback_ready: (Mutex<bool>, Condvar),
    /// Persistent subscriber connection for pushing event notifications.
    pub subscriber: Mutex<Option<BufWriter<UnixStream>>>,
    /// Current review status, updated by main thread.
    pub review_status: Mutex<ReviewStatus>,
}

impl McpChannelState {
    fn new() -> Self {
        Self {
            feedback: Mutex::new(None),
            feedback_ready: (Mutex::new(false), Condvar::new()),
            subscriber: Mutex::new(None),
            review_status: Mutex::new(ReviewStatus {
                summary: "No comments yet".to_string(),
                comment_count: 0,
                files_reviewed: 0,
                files_total: 0,
            }),
        }
    }
}

/// Unix domain socket server for MCP channel communication.
pub struct McpChannelServer {
    socket_path: PathBuf,
    state: Arc<McpChannelState>,
    event_tx: Sender<McpChannelEvent>,
}

impl McpChannelServer {
    /// Create a new server. Call `start()` to begin listening.
    pub fn new(repo_root: &Path, event_tx: Sender<McpChannelEvent>) -> Self {
        let socket_path = compute_socket_path(repo_root);
        Self {
            socket_path,
            state: Arc::new(McpChannelState::new()),
            event_tx,
        }
    }

    /// Get a reference to the shared state (for the main thread to read/write).
    pub fn state(&self) -> Arc<McpChannelState> {
        Arc::clone(&self.state)
    }

    /// Get the socket path.
    #[allow(dead_code)]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Start listening on the Unix socket. Returns the join handle for the
    /// accept loop thread, or `None` if binding failed (another instance is running).
    pub fn start(&self) -> Option<JoinHandle<()>> {
        // Check for existing socket
        if self.socket_path.exists() {
            // Try connecting — if it responds, another instance is live
            if UnixStream::connect(&self.socket_path).is_ok() {
                let _ = self.event_tx.send(McpChannelEvent::BindFailed(format!(
                    "Another tuicr instance is already serving MCP channel for this repo ({})",
                    self.socket_path.display()
                )));
                return None;
            }
            // Stale socket from a crashed process — remove it
            let _ = std::fs::remove_file(&self.socket_path);
        }

        let listener = match UnixListener::bind(&self.socket_path) {
            Ok(l) => l,
            Err(e) => {
                let _ = self.event_tx.send(McpChannelEvent::BindFailed(format!(
                    "Failed to bind MCP channel socket: {e}"
                )));
                return None;
            }
        };

        let state = Arc::clone(&self.state);
        let event_tx = self.event_tx.clone();

        let handle = thread::Builder::new()
            .name("mcp-channel-server".to_string())
            .spawn(move || {
                accept_loop(listener, state, event_tx);
            })
            .expect("failed to spawn MCP channel server thread");

        Some(handle)
    }

    /// Remove the socket file. Call on shutdown and in the panic hook.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Compute the socket path: `/tmp/tuicr-<sha256_first_12_hex>.sock`
///
/// Matches monocle's convention so channel.ts can compute the same path.
pub fn compute_socket_path(repo_root: &Path) -> PathBuf {
    let abs = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let hash = Sha256::digest(abs.to_string_lossy().as_bytes());
    let hex = format!("{:x}", hash);
    let short = &hex[..12];
    PathBuf::from(format!("/tmp/tuicr-{short}.sock"))
}

/// Accept loop — runs on a background thread.
fn accept_loop(
    listener: UnixListener,
    state: Arc<McpChannelState>,
    event_tx: Sender<McpChannelEvent>,
) {
    for stream in listener.incoming() {
        match stream {
            Ok(conn) => {
                let state = Arc::clone(&state);
                let event_tx = event_tx.clone();
                thread::spawn(move || {
                    handle_connection(conn, state, event_tx);
                });
            }
            Err(_) => {
                // Listener was closed (shutdown) — exit the loop
                break;
            }
        }
    }
}

/// Handle a single connection. The first message determines if it's a persistent
/// subscription or a one-shot request/response.
fn handle_connection(
    conn: UnixStream,
    state: Arc<McpChannelState>,
    event_tx: Sender<McpChannelEvent>,
) {
    let reader_stream = match conn.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let reader = BufReader::new(reader_stream);
    let mut lines = reader.lines();

    // Read first message
    let first_line = match lines.next() {
        Some(Ok(line)) if !line.trim().is_empty() => line,
        _ => return,
    };

    let msg = match protocol::decode(&first_line) {
        Ok(m) => m,
        Err(_) => return,
    };

    match msg {
        InboundMessage::Subscribe(req) => {
            handle_subscription(conn, lines, req, state, event_tx);
        }
        _ => {
            // One-shot: handle the message, send response, close
            let response = handle_message(&msg, &state);
            if let Some(resp) = response {
                if let Ok(bytes) = protocol::encode(&resp) {
                    let mut writer = BufWriter::new(&conn);
                    let _ = writer.write_all(&bytes);
                    let _ = writer.flush();
                }
            }
            let _ = conn.shutdown(Shutdown::Both);
        }
    }
}

/// Handle a persistent subscription connection.
fn handle_subscription(
    conn: UnixStream,
    lines: impl Iterator<Item = std::io::Result<String>>,
    _req: protocol::SubscribeRequest,
    state: Arc<McpChannelState>,
    event_tx: Sender<McpChannelEvent>,
) {
    let writer = BufWriter::new(conn.try_clone().expect("failed to clone stream for subscriber"));

    // Send ack
    {
        let ack = OutboundMessage::SubscribeResponse { ok: true };
        if let Ok(bytes) = protocol::encode(&ack) {
            let mut w = BufWriter::new(&conn);
            let _ = w.write_all(&bytes);
            let _ = w.flush();
        }
    }

    // Store subscriber for push notifications
    {
        let mut sub = state.subscriber.lock().unwrap();
        *sub = Some(writer);
    }

    // Notify main thread
    let _ = event_tx.send(McpChannelEvent::Connected);

    // Read loop: handle incoming request/response messages
    for line_result in lines {
        let line = match line_result {
            Ok(l) if !l.trim().is_empty() => l,
            Ok(_) => continue,
            Err(_) => break,
        };

        let msg = match protocol::decode(&line) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let response = handle_message(&msg, &state);
        if let Some(resp) = response {
            if let Ok(bytes) = protocol::encode(&resp) {
                let mut sub = state.subscriber.lock().unwrap();
                if let Some(ref mut w) = *sub {
                    if w.write_all(&bytes).is_err() || w.flush().is_err() {
                        break;
                    }
                }
            }
        }
    }

    // Connection closed — clear subscriber
    {
        let mut sub = state.subscriber.lock().unwrap();
        *sub = None;
    }
    let _ = event_tx.send(McpChannelEvent::Disconnected);
}

/// Route a decoded message to the appropriate handler.
fn handle_message(msg: &InboundMessage, state: &McpChannelState) -> Option<OutboundMessage> {
    match msg {
        InboundMessage::GetReviewStatus => {
            let status = state.review_status.lock().unwrap();
            Some(OutboundMessage::GetReviewStatusResponse {
                summary: status.summary.clone(),
                comment_count: status.comment_count,
                files_reviewed: status.files_reviewed,
                files_total: status.files_total,
            })
        }
        InboundMessage::PollFeedback(req) => {
            if req.wait {
                // Blocking wait for feedback
                let (lock, cvar) = &state.feedback_ready;
                let mut ready = lock.lock().unwrap();
                while !*ready {
                    ready = cvar.wait(ready).unwrap();
                }
                *ready = false;
            }

            let mut feedback = state.feedback.lock().unwrap();
            match feedback.take() {
                Some(content) => Some(OutboundMessage::PollFeedbackResponse {
                    has_feedback: true,
                    feedback: Some(content),
                }),
                None => Some(OutboundMessage::PollFeedbackResponse {
                    has_feedback: false,
                    feedback: None,
                }),
            }
        }
        InboundMessage::Subscribe(_) => {
            // Subscribe should only be the first message; ignore if received later
            None
        }
    }
}

/// Push an event notification to the subscriber (if connected).
/// Called from the main TUI thread when feedback is exported.
pub fn notify_subscriber(state: &McpChannelState, event: &str, message: &str) {
    let notification = OutboundMessage::EventNotification {
        event: event.to_string(),
        payload: Some(EventPayload {
            message: message.to_string(),
        }),
    };

    if let Ok(bytes) = protocol::encode(&notification) {
        let mut sub = state.subscriber.lock().unwrap();
        if let Some(ref mut writer) = *sub {
            let _ = writer.write_all(&bytes);
            let _ = writer.flush();
        }
    }
}

/// Submit feedback to the queue and wake any blocking poll_feedback requests.
/// Called from the main TUI thread when the user exports.
pub fn submit_feedback(state: &McpChannelState, content: String) {
    // Store feedback
    {
        let mut feedback = state.feedback.lock().unwrap();
        *feedback = Some(content);
    }

    // Wake blocking waiters
    {
        let (lock, cvar) = &state.feedback_ready;
        let mut ready = lock.lock().unwrap();
        *ready = true;
        cvar.notify_all();
    }

    // Push event notification
    notify_subscriber(
        state,
        "feedback_submitted",
        "Your reviewer has submitted feedback. Use the get_feedback tool to retrieve it.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn should_compute_deterministic_socket_path() {
        let path1 = compute_socket_path(&PathBuf::from("/tmp/test-repo"));
        let path2 = compute_socket_path(&PathBuf::from("/tmp/test-repo"));
        assert_eq!(path1, path2);
    }

    #[test]
    fn should_compute_different_paths_for_different_repos() {
        let path1 = compute_socket_path(&PathBuf::from("/tmp/repo-a"));
        let path2 = compute_socket_path(&PathBuf::from("/tmp/repo-b"));
        assert_ne!(path1, path2);
    }

    #[test]
    fn should_have_correct_socket_path_format() {
        let path = compute_socket_path(&PathBuf::from("/tmp/test-repo"));
        let path_str = path.to_string_lossy();
        assert!(path_str.starts_with("/tmp/tuicr-"));
        assert!(path_str.ends_with(".sock"));
        // Hash should be 12 hex chars
        let name = path.file_stem().unwrap().to_string_lossy();
        let hash_part = name.strip_prefix("tuicr-").unwrap();
        assert_eq!(hash_part.len(), 12);
        assert!(hash_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn should_submit_and_retrieve_feedback() {
        let state = McpChannelState::new();
        submit_feedback(&state, "test feedback".to_string());

        let feedback = state.feedback.lock().unwrap().take();
        assert_eq!(feedback, Some("test feedback".to_string()));
    }
}
