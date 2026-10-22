//! `big-head-view` — browser view surfaces for agent-rendered panels.
//!
//! Hybrid protocol (see `docs/spatial-ui-design.md` §生态参考与协议决策):
//! - **外壳**: 自研 [`protocol::ViewCommand`] (surface + panel content).
//! - **内容**: A2UI v0.9 adjacency list as panel `content` schema.
//! - **传输**: WebSocket JSON lines (default port 3030, configurable).
//!
//! ## Module Layout
//!
//! - [`protocol`] —— `ViewCommand`, `PanelKind`, `PanelContent`, A2UI types.
//! - [`channel`]  —— In-process `ViewChannel` for agent → server command flow.
//! - [`server`]   —— axum HTTP + WebSocket broadcast server.
//!
//! ## 端到端示例
//!
//! ```no_run
//! use big_head_view::{ViewChannel, ViewServer, ViewCommand, ViewSurface, PanelKind, PanelContent};
//!
//! # async fn run() {
//! // 1. 创建双向 channel
//! let (channel, sink) = ViewChannel::new();
//!
//! // 2. 启动 server (后台 task)
//! let server = ViewServer::new(sink, 3030);
//! tokio::spawn(async move { server.run().await });
//!
//! // 3. Agent 推一个 panel
//! channel.send(ViewCommand::Spawn {
//!     id: "demo".into(),
//!     surface: ViewSurface::Spatial3D,
//!     kind: PanelKind::Card,
//!     title: "Hello".into(),
//!     position: [0.0, 0.0, 0.0],
//!     rotation: None,
//!     content: PanelContent::Card {
//!         label: "Score".into(),
//!         value: serde_json::json!(42),
//!         unit: Some("pts".into()),
//!         trend: None,
//!     },
//! });
//! # }
//! ```

pub mod channel;
pub mod display;
pub mod document;
pub use document::{normalize_document, render_document_tool_definition};
pub mod handler;
pub mod protocol;
pub mod server;
pub mod status;

pub use channel::{ViewChannel, ViewEvent, ViewSink};
pub use display::{DisplayIntent, DisplaySpec, display_intent_for_component, display_spec};
pub use handler::{
    VIEW_AGENT_PROMPT_PATCH, ViewToolHandler, normalize_mind_view_input,
    normalize_view_command_input, render_mind_view_tool_definition, render_status_tool_definition,
    render_view_tool_definition,
};
pub use protocol::{
    A2UIComponent, A2UIContent, ChartPoint, ChartSeries, ChartType, ContentPatch, EntityEdge,
    EntityNode, FormField, FormFieldType, HighlightEffect, InspectorEntry, InspectorField,
    InspectorMetric, InspectorStatus, InspectorTone, PanelContent, PanelKind, TrendDirection,
    TrendIndicator, ViewCommand, ViewSurface,
};
pub use server::{ViewServer, ViewSnapshotHandle};
pub use status::{
    StatusCommand, StatusItem, StatusItemControl, StatusItemState, StatusMessage,
    StatusMessageRole, StatusState,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lib_exports_view_command() {
        let cmd = ViewCommand::Despawn { id: "x".into() };
        let s = serde_json::to_string(&cmd).unwrap();
        assert!(s.contains("despawn"));
    }
}
