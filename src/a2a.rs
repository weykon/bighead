//! Local Agent-to-Agent bridge for the Big Head runtime.
//!
//! The HTTP layer only validates and queues messages. The main runtime owns
//! Spark creation, model execution, ECS mutation, and task completion.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};
use uuid::Uuid;

const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_TTL: u8 = 8;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct A2aMessage {
    pub message_id: Option<String>,
    pub conversation_id: Option<String>,
    pub from: String,
    pub to: Option<String>,
    pub kind: String,
    pub intent: Option<String>,
    pub text: String,
    pub reply_to: Option<String>,
    #[serde(default)]
    pub permissions: A2aPermissions,
    #[serde(default)]
    pub refs: Vec<Value>,
    #[serde(default = "default_ttl")]
    pub ttl: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct A2aPermissions {
    #[serde(default = "default_true")]
    pub can_observe: bool,
    #[serde(default = "default_true")]
    pub can_plan: bool,
    #[serde(default)]
    pub can_mutate: bool,
    #[serde(default)]
    pub can_execute: bool,
    #[serde(default)]
    pub can_forward: bool,
}

impl Default for A2aPermissions {
    fn default() -> Self {
        Self {
            can_observe: true,
            can_plan: true,
            can_mutate: false,
            can_execute: false,
            can_forward: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct A2aTaskReply {
    pub task_id: String,
    pub message_id: String,
    pub conversation_id: String,
    pub status: String,
    pub from: String,
    pub to: String,
    pub text: String,
    pub errors: Vec<String>,
}

pub(crate) struct InboundA2a {
    pub task_id: String,
    pub message: A2aMessage,
    pub reply: oneshot::Sender<A2aTaskReply>,
    pub tasks: A2aTaskStore,
}

pub(crate) type A2aTaskStore = Arc<Mutex<BTreeMap<String, A2aTaskReply>>>;

#[derive(Clone)]
struct A2aState {
    world_id: String,
    tx: mpsc::UnboundedSender<InboundA2a>,
    tasks: A2aTaskStore,
}

pub(crate) struct A2aServer {
    pub port: u16,
    pub world_id: String,
    pub tx: mpsc::UnboundedSender<InboundA2a>,
}

impl A2aServer {
    pub(crate) async fn run(self) -> Result<(), String> {
        let state = A2aState {
            world_id: self.world_id,
            tx: self.tx,
            tasks: Arc::new(Mutex::new(BTreeMap::new())),
        };
        let app = Router::new()
            .route("/.well-known/agent-card.json", get(agent_card))
            .route("/health", get(health))
            .route("/a2a/messages", post(send_message))
            .route("/a2a/tasks/:task_id", get(get_task))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(state);
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|e| format!("A2A bind {address}: {e}"))?;
        log::info!("Big Head A2A bridge listening on http://{address}");
        axum::serve(listener, app)
            .await
            .map_err(|e| format!("A2A server: {e}"))
    }
}

async fn health() -> &'static str {
    "ok"
}

async fn agent_card(State(state): State<A2aState>) -> Json<Value> {
    Json(json!({
        "name": "big-head",
        "description": "Bounded local Agent-to-Agent bridge for a sovereign ECS world",
        "world_id": state.world_id,
        "protocol": "big-head-a2a-v1",
        "transport": "http-local",
        "capabilities": ["observe", "research", "plan", "reply"],
        "endpoints": {
            "messages": "/a2a/messages",
            "tasks": "/a2a/tasks/{task_id}",
            "health": "/health"
        },
        "limits": {
            "max_message_bytes": MAX_TEXT_BYTES,
            "max_ttl": MAX_TTL,
            "mutate": false,
            "execute": false,
            "forward": false
        }
    }))
}

async fn get_task(
    State(state): State<A2aState>,
    Path(task_id): Path<String>,
) -> Result<Json<A2aTaskReply>, (StatusCode, String)> {
    let tasks = state.tasks.lock().await;
    tasks.get(&task_id).cloned().map(Json).ok_or((
        StatusCode::NOT_FOUND,
        format!("unknown A2A task: {task_id}"),
    ))
}

async fn send_message(
    State(state): State<A2aState>,
    Json(message): Json<A2aMessage>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    validate(&message, &state.world_id)?;
    let message_id = message
        .message_id
        .clone()
        .unwrap_or_else(|| format!("msg_{}", Uuid::new_v4()));
    let conversation_id = message
        .conversation_id
        .clone()
        .unwrap_or_else(|| format!("conv_{}", Uuid::new_v4()));
    let mut message = message;
    message.message_id = Some(message_id.clone());
    message.conversation_id = Some(conversation_id.clone());
    let task_id = format!("task_{}", Uuid::new_v4());
    let queued = A2aTaskReply {
        task_id: task_id.clone(),
        message_id: message_id.clone(),
        conversation_id: conversation_id.clone(),
        status: "queued".into(),
        from: state.world_id.clone(),
        to: message.from.clone(),
        text: String::new(),
        errors: Vec::new(),
    };
    state
        .tasks
        .lock()
        .await
        .insert(task_id.clone(), queued.clone());

    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(InboundA2a {
            task_id: task_id.clone(),
            message,
            reply: reply_tx,
            tasks: state.tasks.clone(),
        })
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime is not accepting A2A messages".into(),
            )
        })?;

    match tokio::time::timeout(RESPONSE_TIMEOUT, reply_rx).await {
        Ok(Ok(reply)) => {
            state.tasks.lock().await.insert(task_id, reply.clone());
            Ok((StatusCode::OK, Json(reply)))
        }
        Ok(Err(_)) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime dropped A2A task".into(),
        )),
        Err(_) => Ok((StatusCode::ACCEPTED, Json(queued))),
    }
}

fn validate(message: &A2aMessage, world_id: &str) -> Result<(), (StatusCode, String)> {
    for (field, value) in [
        ("from", message.from.as_str()),
        ("kind", message.kind.as_str()),
    ] {
        if value.trim().is_empty() || value.len() > MAX_ID_BYTES {
            return Err((StatusCode::BAD_REQUEST, format!("invalid A2A {field}")));
        }
    }
    if let Some(to) = message.to.as_deref() {
        if to != world_id && to != "*" {
            return Err((
                StatusCode::NOT_FOUND,
                format!("A2A target is not this world: {to}"),
            ));
        }
    }
    if message.text.trim().is_empty() || message.text.len() > MAX_TEXT_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            "A2A text must be 1..16384 bytes".into(),
        ));
    }
    if message.ttl == 0 || message.ttl > MAX_TTL {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("A2A ttl must be 1..{MAX_TTL}"),
        ));
    }
    if message.permissions.can_mutate
        || message.permissions.can_execute
        || message.permissions.can_forward
    {
        return Err((
            StatusCode::FORBIDDEN,
            "this local bridge only permits observe, research, plan, and reply".into(),
        ));
    }
    if message.refs.len() > 32 {
        return Err((StatusCode::BAD_REQUEST, "A2A refs limit is 32".into()));
    }
    Ok(())
}

fn default_true() -> bool {
    true
}

fn default_ttl() -> u8 {
    3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> A2aMessage {
        A2aMessage {
            message_id: None,
            conversation_id: None,
            from: "codex".into(),
            to: Some("world:local-default".into()),
            kind: "request".into(),
            intent: Some("research".into()),
            text: "检查 Coding Agent 能力".into(),
            reply_to: None,
            permissions: A2aPermissions::default(),
            refs: Vec::new(),
            ttl: 3,
        }
    }

    #[test]
    fn accepts_bounded_local_research_message() {
        assert!(validate(&message(), "world:local-default").is_ok());
    }

    #[test]
    fn rejects_mutation_permission_and_wrong_target() {
        let mut message = message();
        message.permissions.can_mutate = true;
        assert_eq!(
            validate(&message, "world:local-default").unwrap_err().0,
            StatusCode::FORBIDDEN
        );
        message.permissions.can_mutate = false;
        message.to = Some("world:other".into());
        assert_eq!(
            validate(&message, "world:local-default").unwrap_err().0,
            StatusCode::NOT_FOUND
        );
    }
}
