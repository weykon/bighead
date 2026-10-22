//! Test render script - sends ViewCommand from ECS to browser
use big_head_view::{
    FormField, FormFieldType, PanelContent, PanelKind, ViewChannel, ViewCommand, ViewServer,
    ViewSurface,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("🚀 Starting Big Head View Test Renderer");
    println!("========================================\n");

    // Create channel and server
    let (channel, sink) = ViewChannel::new();
    let server = ViewServer::new(sink, 3030);

    // Start server in background
    tokio::spawn(async move {
        if let Err(e) = server.run().await {
            eprintln!("Server error: {}", e);
        }
    });

    // Wait for server startup
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    println!("✅ Server started on http://localhost:3030");
    println!("   Open http://localhost:3030 in your browser to see the view!\n");

    // Panel 1: Form (from Entity 134)
    let cmd1 = ViewCommand::Spawn {
        id: "panel-001".into(),
        surface: ViewSurface::Spatial3D,
        kind: PanelKind::Form,
        title: "新建项目表单".into(),
        position: [1.0, 0.5, 0.0],
        rotation: None,
        content: PanelContent::Form {
            fields: vec![
                FormField {
                    name: "name".into(),
                    label: "项目名称".into(),
                    field_type: FormFieldType::Text,
                    placeholder: Some("输入项目名称...".into()),
                    default: None,
                },
                FormField {
                    name: "priority".into(),
                    label: "优先级".into(),
                    field_type: FormFieldType::Select {
                        options: vec!["高".into(), "中".into(), "低".into()],
                    },
                    placeholder: None,
                    default: None,
                },
            ],
            submit_action: Some("agent:on_submit".into()),
        },
    };

    channel.send(cmd1);
    println!("✅ Sent: Form panel (panel-001)");

    // Panel 2: Card
    let cmd2 = ViewCommand::Spawn {
        id: "panel-002".into(),
        surface: ViewSurface::Spatial3D,
        kind: PanelKind::Card,
        title: "测试状态".into(),
        position: [-1.5, 0.5, 0.0],
        rotation: None,
        content: PanelContent::Card {
            label: "当前进度".into(),
            value: serde_json::json!("75%"),
            unit: None,
            trend: None,
        },
    };
    channel.send(cmd2);
    println!("✅ Sent: Card panel (panel-002)");

    // Panel 3: Table
    let cmd3 = ViewCommand::Spawn {
        id: "panel-003".into(),
        surface: ViewSurface::Spatial3D,
        kind: PanelKind::Table,
        title: "Agent 状态".into(),
        position: [0.0, -1.0, 0.0],
        rotation: None,
        content: PanelContent::Table {
            columns: vec!["Agent".into(), "Status".into(), "Focus".into()],
            rows: vec![
                vec![
                    serde_json::json!("Coordinator"),
                    serde_json::json!("active"),
                    serde_json::json!("main"),
                ],
                vec![
                    serde_json::json!("Agent-2"),
                    serde_json::json!("idle"),
                    serde_json::json!("search"),
                ],
                vec![
                    serde_json::json!("Agent-3"),
                    serde_json::json!("processing"),
                    serde_json::json!("coding"),
                ],
            ],
        },
    };
    channel.send(cmd3);
    println!("✅ Sent: Table panel (panel-003)");

    println!("\n========================================");
    println!("🎉 All panels sent! Check http://localhost:3030");
    println!("   Drag panels to reposition them!");
    println!("   Use mouse wheel to zoom, drag to rotate view.");
    println!("\n   Press Ctrl+C to exit...\n");

    // Keep running
    tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
    Ok(())
}
