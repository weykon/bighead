//! End-to-end integration test: ViewChannel → ViewServer → WebSocket client.
//!
//! Verifies the full pipeline (without an actual browser). We:
//! 1. Create a `ViewChannel`.
//! 2. Start `ViewServer` on a random port.
//! 3. Connect a `tokio_tungstenite` WS client.
//! 4. Send a `Spawn` command via the channel.
//! 5. Assert the client receives the JSON.

use std::time::Duration;

use big_head_view::{PanelContent, PanelKind, ViewChannel, ViewCommand, ViewServer, ViewSurface};
use futures::StreamExt;
use tokio::net::TcpListener;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_text(ws: &mut Ws) -> String {
    let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("ws message within 3s")
        .expect("ws stream item")
        .expect("ws message ok");
    msg.into_text().expect("text frame")
}

#[tokio::test]
async fn e2e_channel_to_browser_ws() {
    // Pick a free port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let (channel, sink) = ViewChannel::new();
    let server = ViewServer::new(sink, port);
    let server_handle = tokio::spawn(async move {
        let _ = server.run_with_global_sink().await;
    });

    // Give the server a tick to bind
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connect WS client
    let url = format!("ws://127.0.0.1:{port}/ws");
    let (mut ws, _response) = connect_async(url).await.expect("ws connect");

    // Wait briefly for connection
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Send a Spawn command via the channel
    channel.send(ViewCommand::Spawn {
        id: "e2e-panel".into(),
        surface: ViewSurface::Spatial3D,
        kind: PanelKind::Card,
        title: "E2E Test".into(),
        position: [1.0, 2.0, 3.0],
        rotation: None,
        content: PanelContent::Card {
            label: "Hello".into(),
            value: serde_json::json!("world"),
            unit: None,
            trend: None,
        },
    });

    let text = next_text(&mut ws).await;
    let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");

    assert_eq!(v["op"], "spawn");
    assert_eq!(v["id"], "e2e-panel");
    assert_eq!(v["kind"], "card");
    assert_eq!(v["position"][2], 3.0);
    assert_eq!(v["content"]["label"], "Hello");

    // Cleanup
    let _ = ws.close(None).await;
    server_handle.abort();
}

#[tokio::test]
async fn e2e_despawn_removes_panel() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let (channel, sink) = ViewChannel::new();
    let server = ViewServer::new(sink, port);
    let server_handle = tokio::spawn(async move {
        let _ = server.run_with_global_sink().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let url = format!("ws://127.0.0.1:{port}/ws");
    let (mut ws, _) = connect_async(url).await.expect("ws connect");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Spawn then despawn
    channel.send(ViewCommand::Spawn {
        id: "x".into(),
        surface: ViewSurface::Spatial3D,
        kind: PanelKind::Card,
        title: "X".into(),
        position: [0.0; 3],
        rotation: None,
        content: PanelContent::Card {
            label: "x".into(),
            value: serde_json::json!(0),
            unit: None,
            trend: None,
        },
    });

    // Drain spawn message
    let j1: serde_json::Value = serde_json::from_str(&next_text(&mut ws).await).unwrap();
    assert_eq!(j1["op"], "spawn");

    // Despawn
    channel.send(ViewCommand::Despawn { id: "x".into() });
    let j2: serde_json::Value = serde_json::from_str(&next_text(&mut ws).await).unwrap();
    assert_eq!(j2["op"], "despawn");
    assert_eq!(j2["id"], "x");

    let _ = ws.close(None).await;
    server_handle.abort();
}
