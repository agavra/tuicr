//! WebSocket server for Claude Code IDE integration.
//!
//! This module implements the MCP (Model Context Protocol) server that
//! allows Claude Code to connect and interact with tuicr.

use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::tungstenite::Message;

use super::handlers;
use super::lockfile::LockFile;
use super::protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use super::state::SharedIdeState;
use super::IdeCommand;

/// Connected client information.
struct Client {
    id: String,
    tx: mpsc::Sender<Message>,
}

/// Server state for managing connected clients.
struct ServerState {
    clients: Vec<Client>,
    next_id: u64,
}

impl ServerState {
    fn new() -> Self {
        Self {
            clients: Vec::new(),
            next_id: 1,
        }
    }

    fn add_client(&mut self, tx: mpsc::Sender<Message>) -> String {
        let id = format!("client-{}", self.next_id);
        self.next_id += 1;
        self.clients.push(Client { id: id.clone(), tx });
        id
    }

    fn remove_client(&mut self, id: &str) {
        self.clients.retain(|c| c.id != id);
    }

    #[allow(dead_code)]
    async fn broadcast(&self, message: &str) {
        for client in &self.clients {
            let _ = client.tx.send(Message::Text(message.to_string())).await;
        }
    }
}

type SharedServerState = Arc<RwLock<ServerState>>;

/// IDE server that handles WebSocket connections from Claude Code.
pub struct IdeServer {
    port: u16,
    ide_state: SharedIdeState,
    server_state: SharedServerState,
    command_tx: mpsc::Sender<IdeCommand>,
    _lock_file: Option<LockFile>,
}

impl IdeServer {
    /// Create a new IDE server (but don't start it yet).
    pub fn new(ide_state: SharedIdeState, command_tx: mpsc::Sender<IdeCommand>) -> Self {
        Self {
            port: 0, // Will be assigned when started
            ide_state,
            server_state: Arc::new(RwLock::new(ServerState::new())),
            command_tx,
            _lock_file: None,
        }
    }

    /// Start the WebSocket server and return the port it's listening on.
    pub async fn start(&mut self, workspace_path: &str) -> Result<u16, ServerError> {
        // Bind to a random available port on localhost
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| ServerError::Bind(e.to_string()))?;

        let addr = listener
            .local_addr()
            .map_err(|e| ServerError::Bind(e.to_string()))?;

        self.port = addr.port();

        // Create lock file for Claude Code discovery
        let lock_file = LockFile::create(self.port, workspace_path)
            .await
            .map_err(|e| ServerError::LockFile(e.to_string()))?;

        self._lock_file = Some(lock_file);

        // Spawn the server accept loop
        let ide_state = self.ide_state.clone();
        let server_state = self.server_state.clone();
        let command_tx = self.command_tx.clone();

        tokio::spawn(async move {
            accept_loop(listener, ide_state, server_state, command_tx).await;
        });

        Ok(self.port)
    }

    /// Get the port the server is listening on.
    #[allow(dead_code)]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Broadcast a notification to all connected clients.
    #[allow(dead_code)]
    pub async fn broadcast_notification(&self, method: &str, params: Option<serde_json::Value>) {
        let notification = JsonRpcNotification::new(method, params);
        if let Ok(json) = serde_json::to_string(&notification) {
            let state = self.server_state.read().await;
            state.broadcast(&json).await;
        }
    }

    /// Get the number of connected clients.
    #[allow(dead_code)]
    pub async fn client_count(&self) -> usize {
        let state = self.server_state.read().await;
        state.clients.len()
    }
}

/// Server accept loop - accepts new WebSocket connections.
async fn accept_loop(
    listener: TcpListener,
    ide_state: SharedIdeState,
    server_state: SharedServerState,
    command_tx: mpsc::Sender<IdeCommand>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let ide_state = ide_state.clone();
                let server_state = server_state.clone();
                let command_tx = command_tx.clone();

                tokio::spawn(async move {
                    if let Err(e) =
                        handle_connection(stream, addr, ide_state, server_state, command_tx).await
                    {
                        eprintln!("IDE connection error: {e}");
                    }
                });
            }
            Err(e) => {
                eprintln!("IDE server accept error: {e}");
            }
        }
    }
}

/// Handle a single WebSocket connection.
async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    ide_state: SharedIdeState,
    server_state: SharedServerState,
    command_tx: mpsc::Sender<IdeCommand>,
) -> Result<(), ConnectionError> {
    // Upgrade to WebSocket
    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|e| ConnectionError::Handshake(e.to_string()))?;

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Create a channel for sending messages to this client
    let (client_tx, mut client_rx) = mpsc::channel::<Message>(32);

    // Register the client
    let client_id = {
        let mut state = server_state.write().await;
        state.add_client(client_tx)
    };

    // Spawn a task to forward messages from the channel to the WebSocket
    let sender_task = tokio::spawn(async move {
        while let Some(msg) = client_rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages
    while let Some(result) = ws_receiver.next().await {
        match result {
            Ok(Message::Text(text)) => {
                if let Some(response) =
                    handle_message(&text, &ide_state, &command_tx).await
                {
                    // Get the client's sender
                    let state = server_state.read().await;
                    if let Some(client) = state.clients.iter().find(|c| c.id == client_id) {
                        let _ = client.tx.send(Message::Text(response)).await;
                    }
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(data)) => {
                let state = server_state.read().await;
                if let Some(client) = state.clients.iter().find(|c| c.id == client_id) {
                    let _ = client.tx.send(Message::Pong(data)).await;
                }
            }
            Ok(_) => {} // Ignore other message types
            Err(e) => {
                eprintln!("WebSocket receive error from {addr}: {e}");
                break;
            }
        }
    }

    // Cleanup
    {
        let mut state = server_state.write().await;
        state.remove_client(&client_id);
    }

    sender_task.abort();
    Ok(())
}

/// Handle a JSON-RPC message and return an optional response.
async fn handle_message(
    text: &str,
    ide_state: &SharedIdeState,
    command_tx: &mpsc::Sender<IdeCommand>,
) -> Option<String> {
    // Parse the JSON-RPC request
    let request: JsonRpcRequest = match serde_json::from_str(text) {
        Ok(req) => req,
        Err(e) => {
            let error_response = JsonRpcResponse::error(None, JsonRpcError::parse_error());
            return Some(serde_json::to_string(&error_response).unwrap());
        }
    };

    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        let error_response = JsonRpcResponse::error(
            request.id.clone(),
            JsonRpcError::invalid_request("Expected jsonrpc version 2.0"),
        );
        return Some(serde_json::to_string(&error_response).unwrap());
    }

    // Handle the method
    let response =
        handlers::handle_method(&request.method, request.params, request.id, ide_state, command_tx)
            .await?;

    Some(serde_json::to_string(&response).unwrap())
}

#[derive(Debug)]
pub enum ServerError {
    Bind(String),
    LockFile(String),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bind(msg) => write!(f, "Failed to bind server: {msg}"),
            Self::LockFile(msg) => write!(f, "Lock file error: {msg}"),
        }
    }
}

impl std::error::Error for ServerError {}

#[derive(Debug)]
pub enum ConnectionError {
    Handshake(String),
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Handshake(msg) => write!(f, "WebSocket handshake failed: {msg}"),
        }
    }
}

impl std::error::Error for ConnectionError {}
