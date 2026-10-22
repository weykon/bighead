//! axum HTTP + WebSocket server that broadcasts `ViewCommand` to browsers.
//!
//! Architecture:
//! - HTTP server on `http://localhost:{port}` (default 3030)
//! - `/ws` WebSocket endpoint: browsers subscribe to ViewCommand stream
//! - `/` static index.html (Vite-built R3F frontend, see `frontend/`)
//! - Background task drains `ViewSink` and pushes commands to a `tokio::sync::broadcast`
//!
//! Flow:
//! ```text
//! Agent (LLM tool call) → ViewChannel.send(cmd)
//!        ↓
//! ViewServer.run()  ←  ViewSink.recv_command()
//!        ↓
//! broadcast::Sender.send(cmd_json)
//!        ↓
//! All connected WebSockets forward JSON to browser
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path, State},
    response::IntoResponse,
    routing::get,
};
use futures::SinkExt;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, broadcast};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use crate::channel::{ViewEvent, ViewSink};
use crate::protocol::{ContentPatch, PanelContent, PanelKind, ViewCommand, ViewSurface};
use crate::status::{StatusCommand, StatusState};

fn frontend_files(dist: impl AsRef<std::path::Path>) -> ServeDir {
    ServeDir::new(dist)
}

/// Server state shared across all axum handlers.
#[derive(Clone)]
struct AppState {
    cmd_tx: broadcast::Sender<String>,
    status_tx: broadcast::Sender<String>,
    /// Server-side snapshot of all live panels. Each `ViewCommand` is mirrored
    /// here so reconnecting browsers can hydrate from server truth rather than
    /// accumulating state from the broadcast stream (which they may have
    /// joined late or missed while disconnected).
    snapshot: Arc<AsyncMutex<HashMap<String, StoredPanel>>>,
    status_snapshot: Arc<AsyncMutex<StatusState>>,
}

/// One panel's worth of persistent state. We store the same fields as
/// `ViewCommand::Spawn` plus content, which is enough to re-emit a fresh
/// `Spawn` on client reconnect.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredPanel {
    id: String,
    #[serde(default)]
    surface: ViewSurface,
    kind: PanelKind,
    title: String,
    position: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rotation: Option<[f32; 3]>,
    content: PanelContent,
}

impl StoredPanel {
    fn to_spawn(&self) -> ViewCommand {
        ViewCommand::Spawn {
            id: self.id.clone(),
            surface: self.surface.clone(),
            kind: self.kind.clone(),
            title: self.title.clone(),
            position: self.position,
            rotation: self.rotation,
            content: self.content.clone(),
        }
    }
}

/// Apply one `ViewCommand` to the in-memory snapshot map.
///
/// Sync (no `.await`) so the entire command — including all sub-commands of a
/// `Batch` — is applied atomically under a single lock acquisition. Unknown
/// panel IDs are logged but not fatal: the LLM may `Despawn` something the
/// snapshot never saw (e.g. a panel from a previous run before restart).
fn apply_to_snapshot(map: &mut HashMap<String, StoredPanel>, cmd: &ViewCommand) {
    match cmd {
        ViewCommand::Spawn {
            id,
            surface,
            kind,
            title,
            position,
            rotation,
            content,
        } => {
            map.insert(
                id.clone(),
                StoredPanel {
                    id: id.clone(),
                    surface: surface.clone(),
                    kind: kind.clone(),
                    title: title.clone(),
                    position: *position,
                    rotation: *rotation,
                    content: content.clone(),
                },
            );
        }
        ViewCommand::Update { id, patch } => {
            let Some(panel) = map.get_mut(id) else {
                log::warn!("snapshot: Update for unknown panel '{id}'");
                return;
            };
            match patch {
                ContentPatch::Full { content } => {
                    panel.content = content.clone();
                }
                ContentPatch::DataModel { path, .. } => {
                    // DataModel patches target nested values inside PanelContent
                    // (a serde-tagged enum). Re-applying them to our cached copy
                    // requires turning PanelContent back into Value, applying
                    // the JSON pointer, and re-serializing — error-prone for
                    // v1. We log a warning so it's visible in the agent's
                    // stream; reconnects will see pre-patch content until the
                    // next `Full` patch lands.
                    log::warn!(
                        "snapshot: DataModel patch on '{id}' path={path} not mirrored; \
                         reconnect will see pre-patch content"
                    );
                }
            }
        }
        ViewCommand::Move {
            id,
            position,
            rotation,
            ..
        } => {
            let Some(panel) = map.get_mut(id) else {
                log::warn!("snapshot: Move for unknown panel '{id}'");
                return;
            };
            panel.position = *position;
            if let Some(r) = rotation {
                panel.rotation = Some(*r);
            }
        }
        ViewCommand::Scale { id, scale } => {
            // `ViewCommand::Spawn` has no `scale` field — Scale is applied
            // renderer-side only. We log at debug and don't track.
            if let Some(panel) = map.get(id) {
                log::debug!(
                    "snapshot: Scale on '{}' = {scale:?} not persisted (no Spawn field)",
                    panel.id
                );
            } else {
                log::warn!("snapshot: Scale for unknown panel '{id}'");
            }
        }
        ViewCommand::Despawn { id } => {
            map.remove(id);
        }
        ViewCommand::Batch { commands } => {
            for sub in commands {
                apply_to_snapshot(map, sub);
            }
        }
        ViewCommand::Highlight { .. } => {
            // Transient visual effect with a wall-clock `duration_ms`. Not
            // meaningful to persist — the deadline would be wrong after even
            // a short delay.
        }
    }
}

/// Opaque handle to the panel snapshot for persistence across restarts.
///
/// Obtain via [`ViewServer::snapshot_handle`] before spawning the server.
/// `save()` serializes the current panel set to disk; `load()` reads it
/// back and returns the [`ViewCommand::Spawn`] commands needed to
/// re-broadcast to connected browsers.
pub struct ViewSnapshotHandle {
    inner: Arc<AsyncMutex<HashMap<String, StoredPanel>>>,
}

impl ViewSnapshotHandle {
    /// Serialize current panel snapshot to a JSON file.
    pub async fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let map = self.inner.lock().await;
        let json = serde_json::to_string_pretty(&*map).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    /// Load panel snapshot from JSON and return spawn commands for
    /// re-broadcast. The in-memory snapshot is also replaced.
    pub async fn load(&self, path: &std::path::Path) -> Result<Vec<ViewCommand>, String> {
        let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut map: HashMap<String, StoredPanel> =
            serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if repair_legacy_mind_view_markup(&mut map) {
            let repaired = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
            std::fs::write(path, repaired).map_err(|e| e.to_string())?;
            log::warn!("repaired malformed JavaScript in persisted mind-view v10 snapshot");
        }
        let spawns: Vec<ViewCommand> = map.values().map(|p| p.to_spawn()).collect();
        let mut guard = self.inner.lock().await;
        *guard = map;
        Ok(spawns)
    }
}

fn repair_legacy_mind_view_markup(map: &mut HashMap<String, StoredPanel>) -> bool {
    const MALFORMED: &str = "return{i,d:-P(b[2],0,b[3]).z;}}";
    const REPAIRED: &str = "return{i:i,d:-P(b[2],0,b[3]).z};}";

    let Some(panel) = map.get_mut("mind-view") else {
        return false;
    };
    let PanelContent::Html { markup } = &mut panel.content else {
        return false;
    };
    if !markup.contains(MALFORMED) {
        return false;
    }

    *markup = markup.replace(MALFORMED, REPAIRED);
    true
}

/// `ViewServer` —— owns the port and the dispatch task.
pub struct ViewServer {
    sink: ViewSink,
    port: u16,
    /// Capacity of internal broadcast channel. Each command = 1 slot.
    broadcast_capacity: usize,
    /// Server-side panel snapshot. Created eagerly so `main.rs` can
    /// persist it across restarts.
    snapshot: Arc<AsyncMutex<HashMap<String, StoredPanel>>>,
    /// Lightweight status snapshot for menu bar / widget clients.
    status_snapshot: Arc<AsyncMutex<StatusState>>,
}

impl ViewServer {
    pub fn new(sink: ViewSink, port: u16) -> Self {
        Self {
            sink,
            port,
            broadcast_capacity: 256,
            snapshot: Arc::new(AsyncMutex::new(HashMap::new())),
            status_snapshot: Arc::new(AsyncMutex::new(StatusState::default())),
        }
    }

    /// Override broadcast capacity (default 256).
    pub fn with_broadcast_capacity(mut self, cap: usize) -> Self {
        self.broadcast_capacity = cap;
        self
    }

    /// Return an opaque handle that can persist / restore the panel
    /// snapshot across restarts. Call before moving `self` into
    /// `run()` or `run_with_global_sink()`.
    pub fn snapshot_handle(&self) -> ViewSnapshotHandle {
        ViewSnapshotHandle {
            inner: self.snapshot.clone(),
        }
    }

    /// Run the server. Blocks until shutdown (or error).
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (cmd_tx, _) = broadcast::channel::<String>(self.broadcast_capacity);
        let (status_tx, _) = broadcast::channel::<String>(self.broadcast_capacity);
        let state = AppState {
            cmd_tx: cmd_tx.clone(),
            status_tx: status_tx.clone(),
            snapshot: self.snapshot.clone(),
            status_snapshot: self.status_snapshot.clone(),
        };

        // Task 1: drain ViewSink → broadcast channel + mirror to snapshot
        let sink = self.sink.clone();
        let status_sink = self.sink.clone();
        let dispatch_handle = tokio::spawn(async move {
            dispatch_loop(sink, cmd_tx, self.snapshot.clone()).await;
        });
        let status_dispatch_handle = tokio::spawn(async move {
            status_dispatch_loop(status_sink, status_tx, self.status_snapshot.clone()).await;
        });

        // Task 2: axum HTTP + WebSocket
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .route("/status/ws", get(status_ws_handler))
            .route("/api/snapshot", get(snapshot_handler))
            .route("/api/status", get(status_handler))
            .route("/api/panels/:id", get(panel_handler))
            .route("/health", get(health_handler))
            .with_state(state)
            // Serve frontend dist (built by Vite). Path relative to workspace root.
            .fallback_service(frontend_files("crates/big-head-view/frontend/dist"))
            .layer(CorsLayer::permissive());

        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], self.port));
        log::info!("big-head-view server listening on http://{addr}");
        log::info!("WebSocket: ws://{addr}/ws");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        let serve_result = axum::serve(listener, app).await;

        // If server exits, cancel dispatch task
        dispatch_handle.abort();
        status_dispatch_handle.abort();
        serve_result.map_err(|e| e.into())
    }
}

/// Apply a `ViewCommand` to the snapshot and broadcast to all subscribers.
///
/// Single chokepoint for "emit a ViewCommand". Two paths feed in:
/// - `dispatch_loop` for LLM-originated commands (via ViewSink)
/// - `client_connection` for user-drag-synthesized `Move` commands
///
/// Keeping the logic in one place ensures the snapshot is always in sync
/// with what subscribers see, regardless of origin.
async fn emit_view_command(
    cmd: &ViewCommand,
    cmd_tx: &broadcast::Sender<String>,
    snapshot: &Arc<AsyncMutex<HashMap<String, StoredPanel>>>,
) {
    // 1. Mirror into snapshot first. If broadcast has no subscribers (no
    //    browser open), the snapshot is still up to date so a future
    //    reconnect hydrates correctly.
    {
        let mut map = snapshot.lock().await;
        apply_to_snapshot(&mut map, cmd);
    }

    // 2. Serialize and broadcast. send() returns Err if no subscribers —
    //    that's fine, the browser may not be open.
    if let Ok(json) = serde_json::to_string(cmd) {
        let _ = cmd_tx.send(json);
    } else {
        log::error!("ViewCommand serialize failed (op={:?})", cmd);
    }
}

/// Dispatch loop: ViewSink → emit().
async fn dispatch_loop(
    sink: ViewSink,
    cmd_tx: broadcast::Sender<String>,
    snapshot: Arc<AsyncMutex<HashMap<String, StoredPanel>>>,
) {
    loop {
        match sink.recv_command().await {
            Some(cmd) => emit_view_command(&cmd, &cmd_tx, &snapshot).await,
            None => {
                log::info!("view sink closed, dispatch loop exiting");
                break;
            }
        }
    }
}

async fn emit_status_command(
    cmd: StatusCommand,
    status_tx: &broadcast::Sender<String>,
    snapshot: &Arc<AsyncMutex<StatusState>>,
) {
    {
        let mut state = snapshot.lock().await;
        cmd.clone().apply_to(&mut state);
    }

    if let Ok(json) = serde_json::to_string(&cmd) {
        let _ = status_tx.send(json);
    } else {
        log::error!("StatusCommand serialize failed (op={:?})", cmd);
    }
}

async fn status_dispatch_loop(
    sink: ViewSink,
    status_tx: broadcast::Sender<String>,
    snapshot: Arc<AsyncMutex<StatusState>>,
) {
    loop {
        match sink.recv_status_command().await {
            Some(cmd) => emit_status_command(cmd, &status_tx, &snapshot).await,
            None => {
                log::info!("status sink closed, dispatch loop exiting");
                break;
            }
        }
    }
}

/// Build a `ViewCommand::Move` from a renderer-side drag_end event.
///
/// Exposed (not inlined) so we can unit-test the drag path without a WebSocket.
fn move_from_drag_end(panel_id: String, position: [f32; 3]) -> ViewCommand {
    ViewCommand::Move {
        id: panel_id,
        position,
        rotation: None,
        duration_ms: None,
    }
}

/// WebSocket handler. Upgrade → subscribe to broadcast.
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| client_connection(socket, state))
}

async fn status_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| status_client_connection(socket, state))
}

/// Per-client connection. Forwards broadcast → socket, and form-submit events → sink.
async fn client_connection(socket: WebSocket, state: AppState) {
    let sink = get_global_sink();
    let (mut sender, mut receiver) = socket.split();

    // Hydrate new client from the server-side snapshot BEFORE subscribing to
    // live updates. We send a `Batch { commands: [Spawn × N] }` so the
    // frontend applies one combined command (single reducer tick).
    //
    // Tiny race window: a Spawn that lands between the snapshot read and
    // `subscribe()` is delivered twice (once via Batch, once via broadcast).
    // The frontend's applyCommand is idempotent for same-id Spawn, so this
    // is benign — at worst a re-render with the same data.
    {
        let map = state.snapshot.lock().await;
        if !map.is_empty() {
            let commands: Vec<ViewCommand> = map.values().map(StoredPanel::to_spawn).collect();
            let batch = ViewCommand::Batch { commands };
            match serde_json::to_string(&batch) {
                Ok(json) => {
                    if sender.send(Message::Text(json)).await.is_err() {
                        log::debug!("snapshot hydrate failed (client disconnected)");
                        return;
                    }
                }
                Err(e) => log::error!("snapshot serialize failed: {e}"),
            }
        }
    }

    let mut rx = state.cmd_tx.subscribe();

    // Spawn send-task: broadcast → socket
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Receive-task: socket → ViewEvent
    //
    // DragEnd is handled specially: we synthesize a `ViewCommand::Move` and
    // route it through `emit_view_command` so the snapshot stays in sync
    // and other connected clients see the move. Other events (view_event,
    // ready) go to the agent via the sink for the runtime to
    // process — we don't synthesize commands for them.
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Some(event) = parse_client_event(&text) {
                    match event {
                        ViewEvent::DragEnd { panel_id, position } => {
                            let move_cmd = move_from_drag_end(panel_id, position);
                            emit_view_command(&move_cmd, &state.cmd_tx, &state.snapshot).await;
                        }
                        other => {
                            if let Some(sink) = sink.lock().await.as_ref() {
                                sink.send_event(other);
                            }
                        }
                    }
                }
            }
            Ok(Message::Close(_)) => break,
            Err(e) => {
                log::debug!("ws recv error: {e}");
                break;
            }
            _ => {}
        }
    }
    send_task.abort();
}

async fn status_client_connection(socket: WebSocket, state: AppState) {
    let sink = get_global_sink();
    let (mut sender, mut receiver) = socket.split();

    {
        let snapshot = state.status_snapshot.lock().await.clone();
        let cmd = StatusCommand::SetState { state: snapshot };
        match serde_json::to_string(&cmd) {
            Ok(json) => {
                if sender.send(Message::Text(json)).await.is_err() {
                    log::debug!("status snapshot hydrate failed (client disconnected)");
                    return;
                }
            }
            Err(e) => log::error!("status snapshot serialize failed: {e}"),
        }
    }

    let mut rx = state.status_tx.subscribe();

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(msg) => {
                        if sender.send(Message::Text(msg)).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Text(text))) => {
                        if let Some(event) = parse_status_client_event(&text) {
                            if let Some(sink) = sink.lock().await.as_ref() {
                                sink.send_event(event);
                            }
                        }
                    }
                    Some(Err(e)) => {
                        log::debug!("status ws recv error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
}

fn parse_status_client_event(text: &str) -> Option<ViewEvent> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let ty = v.get("type")?.as_str()?;
    match ty {
        "status_event" => {
            let item_id = v.get("item_id")?.as_str()?.to_string();
            let action = v
                .get("action")
                .and_then(|a| a.as_str())
                .unwrap_or("open")
                .to_string();
            let mut data = match v {
                serde_json::Value::Object(map) => serde_json::Value::Object(map),
                _ => return None,
            };
            if let Some(map) = data.as_object_mut() {
                map.insert(
                    "surface".into(),
                    serde_json::Value::String("menubar".into()),
                );
                map.insert("action".into(), serde_json::Value::String(action));
                map.entry("title").or_insert(serde_json::Value::Null);
            }
            Some(ViewEvent::ViewEvent {
                panel_id: format!("status:{item_id}"),
                data,
            })
        }
        "spark" => {
            let text = v.get("text")?.as_str()?.trim().to_string();
            let has_attachments = v
                .get("attachments")
                .and_then(|attachments| attachments.as_array())
                .is_some_and(|attachments| !attachments.is_empty());
            if text.is_empty() && !has_attachments {
                return None;
            }
            let mut data = match v {
                serde_json::Value::Object(map) => serde_json::Value::Object(map),
                _ => return None,
            };
            if let Some(map) = data.as_object_mut() {
                map.insert(
                    "surface".into(),
                    serde_json::Value::String("desktop".into()),
                );
                map.insert("type".into(), serde_json::Value::String("spark".into()));
                map.insert("text".into(), serde_json::Value::String(text));
            }
            Some(ViewEvent::ViewEvent {
                panel_id: "desktop:input".into(),
                data,
            })
        }
        _ => {
            log::warn!("unknown status client event type: {ty}");
            None
        }
    }
}

/// Parse client → agent event. Expected format:
/// ```json
/// { "type": "view_event", "panel_id": "...", "data": { ... } }
/// { "type": "ready", "client_id": "..." }
/// { "type": "drag_end", "id": "...", "position": [x, y, z] }
/// ```
///
/// `view_event.data` 嘅 schema 由 agent 喺 spawn panel 嗰阵定义 —— server
/// 唔 validate, pass-through 畀 agent 自己解.
fn parse_client_event(text: &str) -> Option<ViewEvent> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let ty = v.get("type")?.as_str()?;
    match ty {
        "view_event" => Some(ViewEvent::ViewEvent {
            panel_id: v.get("panel_id")?.as_str()?.to_string(),
            data: v.get("data").cloned().unwrap_or(serde_json::Value::Null),
        }),
        "drag_end" => {
            let id = v.get("id")?.as_str()?.to_string();
            let pos = v.get("position")?.as_array()?;
            let x = pos.get(0)?.as_f64()? as f32;
            let y = pos.get(1)?.as_f64()? as f32;
            let z = pos.get(2)?.as_f64()? as f32;
            Some(ViewEvent::DragEnd {
                panel_id: id,
                position: [x, y, z],
            })
        }
        "ready" => Some(ViewEvent::Ready {
            client_id: v.get("client_id")?.as_str()?.to_string(),
        }),
        _ => {
            log::warn!("unknown client event type: {ty}");
            None
        }
    }
}

async fn health_handler() -> &'static str {
    "ok"
}

/// `GET /api/snapshot` — return the entire panel snapshot as JSON.
///
/// This is the **frontend truth source** that the agent can read over HTTP
/// without needing WebSocket access. Lets the agent (and any external
/// tooling) introspect exactly what the browser canvas currently has.
///
/// Returns a map of `panel_id → StoredPanel`. Empty map if no panels.
async fn snapshot_handler(State(state): State<AppState>) -> Json<HashMap<String, StoredPanel>> {
    let map = state.snapshot.lock().await;
    Json(map.clone())
}

/// `GET /api/status` — return the lightweight menu bar status snapshot.
///
/// This is the runtime JSON contract for non-agent clients. Native apps can
/// hydrate from this endpoint, then subscribe to `/status/ws` for updates.
async fn status_handler(State(state): State<AppState>) -> Json<StatusState> {
    let state = state.status_snapshot.lock().await;
    Json(state.clone())
}

/// `GET /api/panels/:id` — return a single panel's snapshot as JSON, or
/// 404 if not found. Useful for the agent to fetch one specific panel
/// without dumping the whole snapshot.
async fn panel_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<StoredPanel>, (axum::http::StatusCode, String)> {
    let map = state.snapshot.lock().await;
    match map.get(&id) {
        Some(p) => Ok(Json(p.clone())),
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("panel '{id}' not in snapshot"),
        )),
    }
}

// ─── Global sink (so ws_handler can push events back to agent) ───
//
// The view server is created with a sink, but the ws_handler closure
// doesn't capture it directly. We stash it in a global Mutex<Option<...>>
// when dispatch_loop starts. This is a simplification — for full correctness
// we'd thread the sink through AppState. But for a single-process PoC it works.

static GLOBAL_SINK: OnceCell<Arc<AsyncMutex<Option<ViewSink>>>> = OnceCell::new();

fn get_global_sink() -> Arc<AsyncMutex<Option<ViewSink>>> {
    GLOBAL_SINK
        .get_or_init(|| Arc::new(AsyncMutex::new(None)))
        .clone()
}

// In `run`, we register the sink so ws_handler can find it.
impl ViewServer {
    /// Same as `run` but registers the sink globally so `ws_handler`
    /// can route client events back to the agent.
    pub async fn run_with_global_sink(
        self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Register sink
        {
            let g = get_global_sink();
            let mut guard = g.lock().await;
            *guard = Some(self.sink.clone());
        }
        self.run().await
    }
}

// We need `tokio::sync::Mutex` and `axum::extract::ws::WebSocket::split`.
// The split method requires the futures::StreamExt trait.
use futures::StreamExt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ViewChannel;
    use crate::protocol::{PanelContent, PanelKind};

    #[tokio::test]
    async fn static_files_do_not_expose_workspace_or_parent_files() {
        use axum::body::{Body, to_bytes};
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        struct TestDirectory(std::path::PathBuf);
        impl Drop for TestDirectory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = TestDirectory(std::env::temp_dir().join(format!(
            "big-head-static-files-{}-{unique}",
            std::process::id()
        )));
        let dist = root.0.join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "frontend index").unwrap();
        std::fs::write(dist.join("app.js"), "frontend asset").unwrap();
        std::fs::write(root.0.join(".env"), "TEST_ONLY_SECRET=not-a-real-key").unwrap();
        std::fs::write(root.0.join("private.txt"), "private fixture").unwrap();

        for static_root in [&dist, &root.0.join("missing-dist")] {
            let app = Router::new().fallback_service(frontend_files(static_root));
            for path in [
                "/.env",
                "/private.txt",
                "/Cargo.toml",
                "/../.env",
                "/%2e%2e/private.txt",
            ] {
                let response = app
                    .clone()
                    .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            }
        }

        let app = Router::new().fallback_service(frontend_files(&dist));
        for (path, expected) in [("/", "frontend index"), ("/app.js", "frontend asset")] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
                expected.as_bytes()
            );
        }
    }

    #[test]
    fn parse_view_event_with_form_data() {
        let raw = r#"{"type":"view_event","panel_id":"p1","data":{"name":"alice","age":30}}"#;
        let evt = parse_client_event(raw).expect("parsed");
        match evt {
            ViewEvent::ViewEvent { panel_id, data } => {
                assert_eq!(panel_id, "p1");
                assert_eq!(data["name"], "alice");
                assert_eq!(data["age"], 30);
            }
            _ => panic!("expected ViewEvent"),
        }
    }

    #[test]
    fn parse_view_event_with_action_data() {
        // A2UI button click: data shape 由 agent 喺 spawn 嗰阵定义.
        let raw = r#"{"type":"view_event","panel_id":"p1","data":{"component_id":"btn1","x":1}}"#;
        let evt = parse_client_event(raw).expect("parsed");
        match evt {
            ViewEvent::ViewEvent { panel_id, data } => {
                assert_eq!(panel_id, "p1");
                assert_eq!(data["component_id"], "btn1");
                assert_eq!(data["x"], 1);
            }
            _ => panic!("expected ViewEvent"),
        }
    }

    #[test]
    fn parse_view_event_missing_data_defaults_to_null() {
        let raw = r#"{"type":"view_event","panel_id":"p1"}"#;
        let evt = parse_client_event(raw).expect("parsed");
        match evt {
            ViewEvent::ViewEvent { panel_id, data } => {
                assert_eq!(panel_id, "p1");
                assert_eq!(data, serde_json::Value::Null);
            }
            _ => panic!("expected ViewEvent"),
        }
    }

    #[test]
    fn parse_drag_end_event() {
        let raw = r#"{"type":"drag_end","id":"panel-1","position":[1.5,-0.3,0.0]}"#;
        let evt = parse_client_event(raw).expect("parsed");
        match evt {
            ViewEvent::DragEnd { panel_id, position } => {
                assert_eq!(panel_id, "panel-1");
                assert_eq!(position, [1.5, -0.3, 0.0]);
            }
            _ => panic!("expected DragEnd"),
        }
    }

    #[test]
    fn parse_status_event_routes_to_view_event() {
        let raw = r#"{"type":"status_event","item_id":"task-1","action":"open","title":"Task"}"#;
        let evt = parse_status_client_event(raw).expect("parsed");
        match evt {
            ViewEvent::ViewEvent { panel_id, data } => {
                assert_eq!(panel_id, "status:task-1");
                assert_eq!(data["surface"], "menubar");
                assert_eq!(data["item_id"], "task-1");
                assert_eq!(data["action"], "open");
                assert_eq!(data["title"], "Task");
            }
            _ => panic!("expected ViewEvent"),
        }
    }

    #[test]
    fn parse_status_reference_event_preserves_context() {
        let raw = r#"{"type":"status_event","item_id":"doc-1:entity:E688","action":"open_reference","title":"TopicInsight","document_id":"doc-1","document_title":"Living Note","reference_kind":"entity","reference_id":"E688","reference_title":"TopicInsight"}"#;
        let evt = parse_status_client_event(raw).expect("parsed");
        match evt {
            ViewEvent::ViewEvent { panel_id, data } => {
                assert_eq!(panel_id, "status:doc-1:entity:E688");
                assert_eq!(data["surface"], "menubar");
                assert_eq!(data["type"], "status_event");
                assert_eq!(data["action"], "open_reference");
                assert_eq!(data["document_id"], "doc-1");
                assert_eq!(data["reference_kind"], "entity");
                assert_eq!(data["reference_id"], "E688");
                assert_eq!(data["reference_title"], "TopicInsight");
            }
            _ => panic!("expected ViewEvent"),
        }
    }

    #[test]
    fn parse_status_spark_event_routes_to_view_event() {
        let raw = r#"{"type":"spark","text":" 帮我整理今日任务 ","source_client":"macos_app","input_surface":"desktop_command_bar"}"#;
        let evt = parse_status_client_event(raw).expect("parsed");
        match evt {
            ViewEvent::ViewEvent { panel_id, data } => {
                assert_eq!(panel_id, "desktop:input");
                assert_eq!(data["surface"], "desktop");
                assert_eq!(data["type"], "spark");
                assert_eq!(data["text"], "帮我整理今日任务");
                assert_eq!(data["source_client"], "macos_app");
                assert_eq!(data["input_surface"], "desktop_command_bar");
            }
            _ => panic!("expected ViewEvent"),
        }
    }

    #[test]
    fn parse_status_spark_event_preserves_attachments() {
        let raw = r#"{"type":"spark","text":"","source_client":"macos_app","input_surface":"desktop_command_bar","attachments":[{"id":"att-1","name":"diagram.png","path":"/tmp/diagram.png","mime_type":"image/png","byte_count":42}]}"#;
        let evt = parse_status_client_event(raw).expect("parsed");
        match evt {
            ViewEvent::ViewEvent { panel_id, data } => {
                assert_eq!(panel_id, "desktop:input");
                assert_eq!(data["surface"], "desktop");
                assert_eq!(data["type"], "spark");
                assert_eq!(data["text"], "");
                assert_eq!(data["attachments"][0]["name"], "diagram.png");
                assert_eq!(data["attachments"][0]["mime_type"], "image/png");
                assert_eq!(data["attachments"][0]["byte_count"], 42);
            }
            _ => panic!("expected ViewEvent"),
        }
    }

    #[tokio::test]
    async fn dispatch_loop_broadcasts_command() {
        let (channel, sink) = ViewChannel::new();
        let (tx, mut rx) = broadcast::channel::<String>(16);
        let sink2 = sink.clone();
        let snapshot: Arc<AsyncMutex<HashMap<String, StoredPanel>>> =
            Arc::new(AsyncMutex::new(HashMap::new()));
        let snapshot2 = snapshot.clone();
        tokio::spawn(async move {
            dispatch_loop(sink2, tx, snapshot2).await;
        });

        channel.send(ViewCommand::Spawn {
            id: "p1".into(),
            surface: ViewSurface::Spatial3D,
            kind: PanelKind::Card,
            title: "Test".into(),
            position: [0.0; 3],
            rotation: None,
            content: PanelContent::Card {
                label: "x".into(),
                value: serde_json::json!(1),
                unit: None,
                trend: None,
            },
        });

        let msg = rx.recv().await.expect("got broadcast");
        assert!(msg.contains("\"op\":\"spawn\""));
        assert!(msg.contains("\"id\":\"p1\""));

        // Snapshot mirrors the broadcast
        let map = snapshot.lock().await;
        assert!(map.contains_key("p1"));
        let p = &map["p1"];
        assert_eq!(p.title, "Test");
        assert_eq!(p.position, [0.0; 3]);
    }

    fn spawn_cmd(id: &str, label: &str) -> ViewCommand {
        ViewCommand::Spawn {
            id: id.into(),
            surface: ViewSurface::Spatial3D,
            kind: PanelKind::Card,
            title: format!("T-{id}"),
            position: [0.0; 3],
            rotation: None,
            content: PanelContent::Card {
                label: label.into(),
                value: serde_json::json!(0),
                unit: None,
                trend: None,
            },
        }
    }

    fn card_panel(id: &str, label: &str) -> StoredPanel {
        StoredPanel {
            id: id.into(),
            surface: ViewSurface::Spatial3D,
            kind: PanelKind::Card,
            title: format!("T-{id}"),
            position: [0.0; 3],
            rotation: None,
            content: PanelContent::Card {
                label: label.into(),
                value: serde_json::json!(0),
                unit: None,
                trend: None,
            },
        }
    }

    #[test]
    fn repairs_known_malformed_mind_view_v10_script() {
        let mut map = HashMap::from([(
            "mind-view".to_string(),
            StoredPanel {
                id: "mind-view".into(),
                surface: ViewSurface::Html2D,
                kind: PanelKind::Html,
                title: "v10".into(),
                position: [0.0; 3],
                rotation: None,
                content: PanelContent::Html {
                    markup: "<script>var rows=B.map(function(b,i){return{i,d:-P(b[2],0,b[3]).z;}});</script>".into(),
                },
            },
        )]);

        assert!(repair_legacy_mind_view_markup(&mut map));
        let PanelContent::Html { markup } = &map["mind-view"].content else {
            panic!("expected html panel");
        };
        assert!(markup.contains("return{i:i,d:-P(b[2],0,b[3]).z};}"));
        assert!(!repair_legacy_mind_view_markup(&mut map));
    }

    #[test]
    fn snapshot_spawn_inserts() {
        let mut map = HashMap::new();
        apply_to_snapshot(&mut map, &spawn_cmd("p1", "alpha"));
        assert!(map.contains_key("p1"));
        assert_eq!(map["p1"].title, "T-p1");
    }

    #[test]
    fn snapshot_move_updates_position() {
        let mut map = HashMap::new();
        map.insert("p1".into(), card_panel("p1", "x"));
        apply_to_snapshot(
            &mut map,
            &ViewCommand::Move {
                id: "p1".into(),
                position: [1.5, -2.0, 3.5],
                rotation: None,
                duration_ms: None,
            },
        );
        assert_eq!(map["p1"].position, [1.5, -2.0, 3.5]);
    }

    #[test]
    fn snapshot_update_full_replaces_content() {
        let mut map = HashMap::new();
        map.insert("p1".into(), card_panel("p1", "old"));
        apply_to_snapshot(
            &mut map,
            &ViewCommand::Update {
                id: "p1".into(),
                patch: ContentPatch::Full {
                    content: PanelContent::Card {
                        label: "new".into(),
                        value: serde_json::json!(99),
                        unit: None,
                        trend: None,
                    },
                },
            },
        );
        match &map["p1"].content {
            PanelContent::Card { label, value, .. } => {
                assert_eq!(label, "new");
                assert_eq!(value, &serde_json::json!(99));
            }
            _ => panic!("expected Card content"),
        }
    }

    #[test]
    fn snapshot_despawn_removes() {
        let mut map = HashMap::new();
        map.insert("p1".into(), card_panel("p1", "x"));
        apply_to_snapshot(&mut map, &ViewCommand::Despawn { id: "p1".into() });
        assert!(!map.contains_key("p1"));
    }

    #[test]
    fn snapshot_batch_applies_all_atomically() {
        let mut map = HashMap::new();
        apply_to_snapshot(
            &mut map,
            &ViewCommand::Batch {
                commands: vec![
                    spawn_cmd("p1", "a"),
                    spawn_cmd("p2", "b"),
                    ViewCommand::Despawn { id: "p1".into() },
                ],
            },
        );
        assert!(!map.contains_key("p1"));
        assert!(map.contains_key("p2"));
    }

    #[test]
    fn snapshot_highlight_skipped() {
        let mut map = HashMap::new();
        map.insert("p1".into(), card_panel("p1", "x"));
        apply_to_snapshot(
            &mut map,
            &ViewCommand::Highlight {
                id: "p1".into(),
                effect: crate::protocol::HighlightEffect::Pulse {
                    color: "#fff".into(),
                },
                duration_ms: 500,
            },
        );
        // Panel still present, unchanged
        assert!(map.contains_key("p1"));
    }

    #[test]
    fn snapshot_unknown_panel_warns_but_no_panic() {
        let mut map = HashMap::new();
        apply_to_snapshot(
            &mut map,
            &ViewCommand::Move {
                id: "ghost".into(),
                position: [0.0; 3],
                rotation: None,
                duration_ms: None,
            },
        );
        assert!(map.is_empty());
    }

    #[test]
    fn move_from_drag_end_builds_move_command() {
        let cmd = move_from_drag_end("p1".into(), [1.5, -2.0, 0.0]);
        match cmd {
            ViewCommand::Move {
                id,
                position,
                rotation,
                duration_ms,
            } => {
                assert_eq!(id, "p1");
                assert_eq!(position, [1.5, -2.0, 0.0]);
                assert!(rotation.is_none());
                assert!(duration_ms.is_none());
            }
            _ => panic!("expected Move"),
        }
    }

    #[tokio::test]
    async fn drag_end_emits_move_and_updates_snapshot() {
        // Simulate the receive-task path: parse drag_end, synthesize Move,
        // emit. Verify both broadcast and snapshot are updated.
        let (cmd_tx, mut rx) = broadcast::channel::<String>(16);
        let snapshot: Arc<AsyncMutex<HashMap<String, StoredPanel>>> =
            Arc::new(AsyncMutex::new(HashMap::new()));
        // Pre-populate snapshot with a panel
        {
            let mut map = snapshot.lock().await;
            map.insert("p1".into(), card_panel("p1", "x"));
        }

        // Synthesize a drag_end via the helper
        let move_cmd = move_from_drag_end("p1".into(), [2.0, 3.0, 4.0]);
        emit_view_command(&move_cmd, &cmd_tx, &snapshot).await;

        // Snapshot reflects new position
        let map = snapshot.lock().await;
        assert_eq!(map["p1"].position, [2.0, 3.0, 4.0]);

        // Broadcast was sent
        let msg = rx.recv().await.expect("got broadcast");
        assert!(msg.contains("\"op\":\"move\""));
        assert!(msg.contains("\"id\":\"p1\""));
        assert!(msg.contains("2.0") && msg.contains("3.0") && msg.contains("4.0"));
    }
}
