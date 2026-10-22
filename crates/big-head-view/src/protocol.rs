//! `ViewCommand` —— agent 推到 external renderer 嘅 command envelope.
//!
//! 设计参考: `docs/spatial-ui-design.md` §协议设计.
//! - 外壳: 自研 `ViewCommand` enum (spatial 一等公民).
//! - 内容: A2UI v0.9 adjacency list (see <https://a2ui.org/concepts/components/>).
//! - 偏离点: 3D position/rotation/scale 喺最外层 (A2UI 假设 2D chat surface).
//!
//! 所有类型都 derive `Serialize + Deserialize + Clone + Debug`,
//! 方便 both wire (JSON over WebSocket) 同 in-process (`tokio::sync::mpsc`) 使用.

use serde::{Deserialize, Serialize};

/// Agent → Renderer 嘅 command envelope.
///
/// 序列化用 `#[serde(tag = "op")]`, 一个 op 一个 JSON object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ViewCommand {
    /// 创建新 panel. 对应 A2UI `createSurface`.
    Spawn {
        id: String,
        #[serde(default)]
        surface: ViewSurface,
        kind: PanelKind,
        title: String,
        /// 3D 空间坐标 [x, y, z].
        position: [f32; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation: Option<[f32; 3]>,
        /// A2UI v0.9 adjacency list (Catalog + components).
        content: PanelContent,
    },
    /// 更新现有 panel 嘅内容. 对应 A2UI `updateComponents` / `updateDataModel`.
    Update {
        id: String,
        /// 增量 patch: `Full` 替换整个 content, `DataModel` 改 A2UI 数据绑定路径.
        patch: ContentPatch,
    },
    /// 移动 panel 喺 3D 空间.
    Move {
        id: String,
        position: [f32; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation: Option<[f32; 3]>,
        /// Optional 动画时长. None 即瞬间跳.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u32>,
    },
    /// 缩放 panel.
    Scale { id: String, scale: [f32; 3] },
    /// 删除 panel. 对应 A2UI `deleteSurface`.
    Despawn { id: String },
    /// 视觉强调 (e.g. 闪烁 / outline / 颜色脉冲).
    Highlight {
        id: String,
        effect: HighlightEffect,
        duration_ms: u32,
    },
    /// 一次推多条 command, 减少 wire round-trip.
    Batch { commands: Vec<ViewCommand> },
}

/// Where a panel should be rendered.
///
/// `Spatial3D` preserves the original floating-canvas behavior. `Html2D`
/// renders the same panel content in a flat DOM surface using the same
/// WebSocket protocol and event bridge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewSurface {
    #[serde(rename = "spatial3d", alias = "spatial3_d")]
    Spatial3D,
    #[serde(rename = "html2d", alias = "html2_d")]
    Html2D,
}

impl Default for ViewSurface {
    fn default() -> Self {
        Self::Spatial3D
    }
}

/// Panel 类型 —— 决定 renderer 边种 template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    /// 多行对比. Template-driven, LLM 只 fill columns/rows.
    Table,
    /// 数值趋势 / 分布 (bar / line / pie / scatter).
    Chart,
    /// 单个 KPI / 状态指标.
    Card,
    /// 收集 user input. Action 回流到 agent.
    Form,
    /// 复杂排版 escape hatch. 直接 render raw HTML inside drei `<Html>`.
    Html,
    /// 嵌套 3D scene (entity 关系图).
    Scene3D,
    /// Raw A2UI adjacency list —— renderer 100% 跟 A2UI v0.9 spec.
    A2ui,
    /// ECS runtime inspector with stable summary, status, and field layout.
    Inspector,
}

/// Panel 内容: 内嵌模板参数 或 raw A2UI adjacency list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PanelContent {
    /// 多行对比 table.
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<serde_json::Value>>,
    },
    /// 数值趋势 chart.
    Chart {
        chart_type: ChartType,
        series: Vec<ChartSeries>,
    },
    /// 单 KPI 卡片.
    Card {
        label: String,
        value: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trend: Option<TrendIndicator>,
    },
    /// 收集 user input.
    Form {
        fields: Vec<FormField>,
        /// 提交时 agent 边度收 (e.g. "agent:on_submit"). 留空默认走 main channel.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        submit_action: Option<String>,
    },
    /// 复杂排版 escape hatch.
    Html { markup: String },
    /// 嵌套 3D scene (entity graph).
    Scene3D {
        /// { entity_id: position }
        nodes: Vec<EntityNode>,
        edges: Vec<EntityEdge>,
    },
    /// Raw A2UI v0.9 adjacency list.
    A2ui(A2UIContent),
    /// Typed ECS runtime inspection. The renderer owns layout and status color;
    /// the agent only supplies a bounded component snapshot.
    Inspector {
        component: String,
        #[serde(default)]
        summary: Vec<InspectorMetric>,
        #[serde(default)]
        entries: Vec<InspectorEntry>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty_message: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectorMetric {
    pub label: String,
    pub value: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<InspectorTone>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectorEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<u64>,
    pub label: String,
    #[serde(default)]
    pub status: InspectorStatus,
    #[serde(default)]
    pub fields: Vec<InspectorField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectorField {
    pub label: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InspectorStatus {
    #[default]
    Idle,
    Healthy,
    Pending,
    Running,
    Warning,
    Error,
    Done,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InspectorTone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

/// Content 增量 patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "patch", rename_all = "snake_case")]
pub enum ContentPatch {
    /// 替换整个 content.
    Full { content: PanelContent },
    /// 改 A2UI 风格嘅数据模型 path (JSON Pointer, RFC 6901).
    /// e.g. 改表格中某 row: path="/rows/2/1", value="new_value"
    DataModel {
        path: String,
        value: serde_json::Value,
    },
}

/// A2UI v0.9 adjacency list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UIContent {
    /// 选 catalog (e.g. "default", "spatial-ext"). Renderer 加载对应 JSON Schema 校验.
    pub catalog_id: String,
    /// Flat list of components with id refs.
    pub components: Vec<A2UIComponent>,
}

/// A2UI v0.9 component —— `{ id, component, props }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UIComponent {
    pub id: String,
    /// Component 名 (e.g. "Column", "Text", "Table", "Button").
    pub component: String,
    /// Component-specific props. 用 `#[serde(flatten)]` 保持 generic.
    #[serde(flatten)]
    pub props: serde_json::Value,
}

/// 图表类型.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChartType {
    Bar,
    Line,
    Pie,
    Scatter,
}

/// Chart 嘅一条 series.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartSeries {
    pub name: String,
    pub data: Vec<ChartPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// 单个数据点.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartPoint {
    pub x: serde_json::Value,
    pub y: serde_json::Value,
}

/// KPI 趋势指示.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendIndicator {
    pub direction: TrendDirection,
    pub magnitude: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Up,
    Down,
    Flat,
}

/// Form field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    pub label: String,
    pub field_type: FormFieldType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldType {
    Text,
    Number,
    Boolean,
    Select { options: Vec<String> },
}

/// 3D scene 节点.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityNode {
    pub entity_id: u64,
    pub label: String,
    pub position: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityEdge {
    pub from: u64,
    pub to: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Highlight 效果.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HighlightEffect {
    Pulse { color: String },
    Outline { color: String, width: f32 },
    Shake { intensity: f32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_serializes_with_op_tag() {
        let cmd = ViewCommand::Spawn {
            id: "p1".into(),
            surface: ViewSurface::Spatial3D,
            kind: PanelKind::Table,
            title: "Status".into(),
            position: [0.0, 0.0, 0.0],
            rotation: None,
            content: PanelContent::Table {
                columns: vec!["A".into(), "B".into()],
                rows: vec![vec![serde_json::json!(1), serde_json::json!("hi")]],
            },
        };
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["op"], "spawn");
        assert_eq!(v["id"], "p1");
        assert_eq!(v["surface"], "spatial3d");
        assert_eq!(v["kind"], "table");
        assert_eq!(v["content"]["columns"][0], "A");
    }

    #[test]
    fn view_surface_uses_compact_wire_names_and_accepts_legacy_names() {
        assert_eq!(
            serde_json::to_value(&ViewSurface::Html2D).unwrap(),
            serde_json::json!("html2d")
        );
        assert_eq!(
            serde_json::to_value(&ViewSurface::Spatial3D).unwrap(),
            serde_json::json!("spatial3d")
        );
        assert_eq!(
            serde_json::from_value::<ViewSurface>(serde_json::json!("html2_d")).unwrap(),
            ViewSurface::Html2D
        );
        assert_eq!(
            serde_json::from_value::<ViewSurface>(serde_json::json!("spatial3_d")).unwrap(),
            ViewSurface::Spatial3D
        );
    }

    #[test]
    fn round_trip_batch() {
        let cmd = ViewCommand::Batch {
            commands: vec![
                ViewCommand::Despawn { id: "old".into() },
                ViewCommand::Spawn {
                    id: "new".into(),
                    surface: ViewSurface::Spatial3D,
                    kind: PanelKind::Card,
                    title: "KPI".into(),
                    position: [1.0, 2.0, 3.0],
                    rotation: None,
                    content: PanelContent::Card {
                        label: "RPM".into(),
                        value: serde_json::json!(1234),
                        unit: Some("ms".into()),
                        trend: None,
                    },
                },
            ],
        };
        let s = serde_json::to_string(&cmd).unwrap();
        let back: ViewCommand = serde_json::from_str(&s).unwrap();
        match back {
            ViewCommand::Batch { commands } => assert_eq!(commands.len(), 2),
            _ => panic!("expected batch"),
        }
    }

    #[test]
    fn a2ui_component_preserves_flattened_props() {
        let comp = A2UIComponent {
            id: "btn1".into(),
            component: "Button".into(),
            props: serde_json::json!({"label": "Click me", "variant": "primary"}),
        };
        let v = serde_json::to_value(&comp).unwrap();
        assert_eq!(v["id"], "btn1");
        assert_eq!(v["component"], "Button");
        assert_eq!(v["label"], "Click me");
        assert_eq!(v["variant"], "primary");
    }

    #[test]
    fn inspector_content_round_trips() {
        let content = PanelContent::Inspector {
            component: "TickAction".into(),
            summary: vec![InspectorMetric {
                label: "active".into(),
                value: serde_json::json!(3),
                tone: Some(InspectorTone::Success),
            }],
            entries: vec![InspectorEntry {
                entity_id: Some(42),
                label: "weather_query".into(),
                status: InspectorStatus::Healthy,
                fields: vec![InspectorField {
                    label: "remaining".into(),
                    value: serde_json::json!(120),
                }],
            }],
            empty_message: None,
        };

        let value = serde_json::to_value(&content).unwrap();
        assert_eq!(value["kind"], "inspector");
        assert_eq!(value["entries"][0]["status"], "healthy");
        let decoded: PanelContent = serde_json::from_value(value).unwrap();
        assert!(matches!(decoded, PanelContent::Inspector { .. }));
    }
}
