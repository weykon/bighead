//! `ViewChannel` —— agent runtime → view server 嘅 in-process channel.
//!
//! 双向:
//! - Agent 推 `ViewCommand` (via `send()`).
//! - Renderer 返 `ViewEvent` (e.g. form submit, button click, drag) (via `recv_events()`).
//!
//! 用 `tokio::sync::mpsc` 简单实现. 冇 backpressure 兜底 —— 假设
//! renderer 跟得上 (前端 R3F 一秒 60 帧, 冇可能积压).
//! 真正压力时用 `tokio::sync::watch` 替代或者加 `try_send` 检查.

use tokio::sync::mpsc;

use crate::protocol::ViewCommand;
use crate::status::StatusCommand;

/// Renderer → Agent 嘅回传事件.
#[derive(Debug, Clone)]
pub enum ViewEvent {
    /// User 喺 panel 上做咗样嘢 (form submit / button click / future
    /// 任何交互). `data` 嘅 schema 由 agent 喺 spawn panel 嗰阵定义
    /// —— server 唔 validate, pass-through 畀 agent 自己解.
    ViewEvent {
        panel_id: String,
        data: serde_json::Value,
    },
    /// User 拖拽面板结束 (3D 特殊语义, server 嗰边直接合成成
    /// `ViewCommand::Move`, 唔经呢个 variant 处理).
    DragEnd {
        panel_id: String,
        position: [f32; 3],
    },
    /// Renderer 喺度 (e.g. browser 加载完成, 想 query 现有 state).
    Ready { client_id: String },
}

/// In-process 双向 channel pair.
#[derive(Debug, Clone)]
pub struct ViewChannel {
    /// Agent → View server (嗰边 broadcast 到 browser).
    pub tx: mpsc::UnboundedSender<ViewCommand>,
    /// Agent → lightweight status surfaces (macOS menu bar, widgets, etc.).
    pub status_tx: mpsc::UnboundedSender<StatusCommand>,
    /// View server → Agent (form submit, action, ready, drag).
    pub rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ViewEvent>>>,
}

impl ViewChannel {
    /// 创建一个 channel pair.
    pub fn new() -> (ViewChannel, ViewSink) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<ViewCommand>();
        let (status_tx, status_rx) = mpsc::unbounded_channel::<StatusCommand>();
        let (evt_tx, evt_rx) = mpsc::unbounded_channel::<ViewEvent>();
        let channel = ViewChannel {
            tx: cmd_tx,
            status_tx,
            rx: Arc::new(tokio::sync::Mutex::new(evt_rx)),
        };
        let sink = ViewSink {
            cmd_rx: Arc::new(tokio::sync::Mutex::new(cmd_rx)),
            status_rx: Arc::new(tokio::sync::Mutex::new(status_rx)),
            evt_tx,
        };
        (channel, sink)
    }

    /// Agent 推一个 command. 失败通常因为 server 已 drop, 静默忽略.
    pub fn send(&self, cmd: ViewCommand) {
        if let Err(e) = self.tx.send(cmd) {
            log::warn!("view channel send failed: {e}");
        }
    }

    /// Agent 推一个 lightweight status command.
    pub fn send_status(&self, cmd: StatusCommand) {
        if let Err(e) = self.status_tx.send(cmd) {
            log::warn!("status channel send failed: {e}");
        }
    }

    /// 异步等下一个 renderer 事件.
    pub async fn recv_event(&self) -> Option<ViewEvent> {
        let mut rx = self.rx.lock().await;
        rx.recv().await
    }
}

/// View server 嘅入站 sink (持有 command rx + event tx).
#[derive(Clone)]
pub struct ViewSink {
    pub(crate) cmd_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ViewCommand>>>,
    pub(crate) status_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<StatusCommand>>>,
    pub(crate) evt_tx: mpsc::UnboundedSender<ViewEvent>,
}

impl ViewSink {
    /// 异步等下一个 agent command.
    pub async fn recv_command(&self) -> Option<ViewCommand> {
        let mut rx = self.cmd_rx.lock().await;
        rx.recv().await
    }

    /// 异步等下一个 agent status command.
    pub async fn recv_status_command(&self) -> Option<StatusCommand> {
        let mut rx = self.status_rx.lock().await;
        rx.recv().await
    }

    /// 推一个 event 畀 agent.
    pub fn send_event(&self, event: ViewEvent) {
        if let Err(e) = self.evt_tx.send(event) {
            log::warn!("view event send failed: {e}");
        }
    }
}

// `Arc` import.
use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PanelContent, PanelKind, ViewSurface};

    #[tokio::test]
    async fn round_trip_command_and_event() {
        let (channel, sink) = ViewChannel::new();
        let sink2 = sink.clone();

        // 启动 consumer
        let consumer = tokio::spawn(async move {
            let cmd = sink2.recv_command().await.expect("got command");
            match cmd {
                ViewCommand::Spawn { id, .. } => assert_eq!(id, "p1"),
                _ => panic!("expected spawn"),
            }
            sink2.send_event(ViewEvent::Ready {
                client_id: "browser1".into(),
            });
        });

        // Producer 推
        channel.send(ViewCommand::Spawn {
            id: "p1".into(),
            surface: ViewSurface::Spatial3D,
            kind: PanelKind::Card,
            title: "Test".into(),
            position: [0.0; 3],
            rotation: None,
            content: PanelContent::Card {
                label: "X".into(),
                value: serde_json::json!(42),
                unit: None,
                trend: None,
            },
        });

        // 等回 event
        let evt = channel.recv_event().await.expect("got event");
        match evt {
            ViewEvent::Ready { client_id } => assert_eq!(client_id, "browser1"),
            _ => panic!("expected ready"),
        }

        consumer.await.unwrap();
    }
}
