# Spatial UI — Agent-Driven Visualization Layer

> Design doc for the new `big-head-view` crate. Status: design phase, not implemented yet.
> 2026-06-03 更新: 完成 6 个生态调研, 协议决策 hybrid (自研 spatial + A2UI v0.9 content schema). 详见 [生态参考与协议决策](#生态参考与协议决策).

## Motivation

现有 `big-head` 系统嘅最终输出系通过 terminal (termimad 渲染 markdown) 展示畀 user。但当 agent 嘅响应包含结构化数据（多行对比、数值趋势、状态指标、entity 关系），纯文本表达有限。

**目标**：畀 agent 一个**主动调用**嘅可视化工具，agent 决定点样呈现数据畀 user，而唔系被动等文本输出。

## 关键概念

### Protocol ↔ Renderer 解耦

```
┌─────────────────────────────────────────────────┐
│  Protocol Layer (agent 嘅接口，稳定)             │
│  - ViewCommand { Spawn, Update, Move, ... }     │
│  - PanelKind { Table, Chart, Card, Form, ... }  │
└──────────────────┬──────────────────────────────┘
                   │ JSON over WebSocket / IPC
                   ▼
┌─────────────────────────────────────────────────┐
│  Render Layer (可替换，user 嘅体验)              │
│  - PoC: HTML + Three.js (3D 空间)               │
│  - 未来: Tauri / Bevy / egui / 纯 HTML          │
└─────────────────────────────────────────────────┘
```

**好处**：换 renderer 唔影响 agent；可以并行多 renderer（A/B 测试、远程 client、多 user）。

### Spatial UI — 3D 空间嘅漂浮 Panels

唔系传统 2D dashboard，而系：
- 3D scene 作 canvas
- 每条 view = 一个漂浮喺空间嘅 panel
- Panel 嘅位置、旋转由 agent 决定
- User 用 OrbitControls 自由导航（拖拽旋转、滚轮 zoom、右键平移）
- Panel 内可嵌套 sub-UI（table / chart / form）
- Agent 有能力 rearrange、animate、highlight panels

## 协议设计

> 经过 6 个生态调研, 决定 hybrid: 自研 spatial extension 协议外壳 + A2UI v0.9 嘅 adjacency list 作为 panel `content` 嘅 schema. 见 [生态参考与协议决策](#生态参考与协议决策) 章节.

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ViewCommand {
    Spawn {
        id: String,
        kind: PanelKind,
        title: String,
        /// 3D 空间坐标 (Spatial extension, A2UI 冇呢个)
        position: [f32; 3],
        rotation: Option<[f32; 3]>,
        /// Panel 内容: A2UI v0.9 嘅 adjacency list (array of components)
        /// 参考 https://a2ui.org/concepts/components/
        content: A2UIContent,
    },
    Update {
        id: String,
        /// 增量 patch: 可以系完整 content 或者 A2UI `updateDataModel` 风格嘅 path/value
        content: ContentPatch,
    },
    Move {
        id: String,
        position: [f32; 3],
        rotation: Option<[f32; 3]>,
        duration_ms: Option<u32>,
    },
    Scale {
        id: String,
        scale: [f32; 3],
    },
    Despawn { id: String },
    Highlight {
        id: String,
        effect: HighlightEffect,
        duration_ms: u32,
    },
    Batch { commands: Vec<ViewCommand> },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    Table,    // template-driven, LLM fill data
    Chart,    // template-driven, LLM fill data
    Card,     // single KPI
    Form,     // collect user input
    Html,     // escape hatch (rendered inside drei <Html>)
    Scene3D,  // 嵌套 3D scene
    A2UI,     // raw A2UI adjacency list, render 100% via A2UI v0.9 spec
}

/// A2UI v0.9 adjacency list: flat list of { id, component, props }.
/// 由 renderer 维护 component tree 重建逻辑.
#[derive(Serialize, Deserialize)]
pub struct A2UIContent {
    pub catalog_id: String,  // 选 catalog (e.g. "default", "spatial-ext")
    pub components: Vec<A2UIComponent>,
}

#[derive(Serialize, Deserialize)]
pub struct A2UIComponent {
    pub id: String,
    pub component: String,  // e.g. "Column", "Text", "Table", "Button"
    #[serde(flatten)]
    pub props: serde_json::Value,
}
```

**PanelKind 决策**: 大部分 `Table` / `Chart` / `Card` 系 *template-driven* (我哋定义 widget 模板, LLM 只 fill data), 减低 LLM 写错 layout 概率 (参考 ChatKit 嘅 `.widget` template 模式). `A2UI` 留 escape hatch 让 LLM 直接 emit 完整 adjacency list.

## 集成架构

```
┌────────────────────────────────────────────────────────────┐
│  Agent (LLM reasoning)                                     │
│      │                                                      │
│      │ tool_call: render_view({                            │
│      │   op: "spawn",                                      │
│      │   id: "tickaction-status",                          │
│      │   kind: "table",                                    │
│      │   title: "TickAction 状态",                         │
│      │   position: [0, 0, 0],                              │
│      │   content: { columns: [...], rows: [...] }          │
│      │ })                                                  │
└──────┬─────────────────────────────────────────────────────┘
       │ ToolExecutor::execute("render_view", input)
       ▼
┌────────────────────────────────────────────────────────────┐
│  big-head-loop 内部:                                       │
│  - RuntimeToolExecutor 新增 match arm: "render_view"        │
│  - 推到 ViewChannel (tokio::sync::mpsc)                    │
└──────┬─────────────────────────────────────────────────────┘
       │ ViewChannel
       ▼
┌────────────────────────────────────────────────────────────┐
│  big-head-view crate (新)                                   │
│  - axum HTTP server on localhost:3030                       │
│  - WebSocket 广播到所有已连接 browser                       │
│  - Static files: index.html, main.tsx, App.tsx             │
└──────┬─────────────────────────────────────────────────────┘
       │ WebSocket (ws://localhost:3030/ws)
       ▼
┌────────────────────────────────────────────────────────────┐
│  Browser (React + R3F + Tailwind)                          │
│  - 收到 view JSON → dispatch 到对应 widget                  │
│  - 多个 view 可同时显示 (sidebar + main + 3D scene)         │
└────────────────────────────────────────────────────────────┘
```

## 6 种 PanelKind

| Kind | LLM 写可靠性 | 适用场景 |
|------|-------------|----------|
| Table | 99% | 多行对比（sparks, work items, orders）|
| Chart | 95% | 数值趋势（bar / line / pie / scatter）|
| Card | 99% | 单个 KPI / 状态指标 |
| Form | 90% | 收集 user input |
| Html | 99% | escape hatch for 复杂排版 |
| Scene3D | 70% | entity 关系 3D 可视化 |

## Tool API

```rust
// ToolExecutor trait 新增方法
pub trait ToolExecutor {
    /// 推一个 view command 到 external view channel
    /// 默认 no-op，唔强制 view crate 必须存在
    fn render_view(&mut self, _view: serde_json::Value) {}
}
```

```rust
// RuntimeToolExecutor 新增 match arm
"render_view" => {
    let cmd: ViewCommand = serde_json::from_value(call.input.clone())
        .map_err(|e| format!("render_view: {}", e))?;
    self.render_view(serde_json::to_value(&cmd).unwrap());
    ("View command sent", false)
}
```

## 系统 Prompt 引导

需要更新 `COORDINATOR_SYSTEM_PROMPT`，加一节关于 `render_view` 嘅使用指引：

```markdown
## Visual Presentation

当你嘅响应包含结构化数据时，可以调用 `render_view` 工具：

- **多行对比数据** → Table
- **数值趋势 / 分布** → Chart
- **单个 KPI** → Card
- **收集 user 输入** → Form
- **空间关系 / entity 网络** → Scene3D
- **复杂排版** → Html (escape hatch)

每个 panel 有 `id`、`kind`、`position` (3D 空间坐标)、
`content` (kind-specific)。`position` 决定 panel 喺 3D 空间
嘅位置——agent 可以有意识咁将相关 panels 摆埋一齐。

例子：
render_view({
  op: "spawn",
  id: "tickactions",
  kind: "table",
  title: "TickAction 状态",
  position: [0, 0, 0],
  content: { columns: ["Entity", "Cooldown", "Last Run"], rows: [...] }
})
```

## 前端技术栈

- **Vite** + **React** + **TypeScript**（build script 输出到 `crates/big-head-view/dist/`）
- **@react-three/fiber** + **@react-three/drei**（3D scene + `<Html>` 漂浮 panel）
- **Tailwind CSS**（styling）
- **WebSocket client** 接收 ViewCommand stream
- **useReducer** 管理 panel state

## 5 步实施路径

| 步骤 | 内容 | 时间 |
|------|------|------|
| 1 | `big-head-view` crate + ViewCommand protocol | 1-2h |
| 2 | `render_view` 工具 + ViewChannel | 1h |
| 3 | axum HTTP server + WebSocket 广播 | 1-2h |
| 4 | Vite + R3F 前端，3D 漂浮 panel | 2-3h |
| 5 | 端到端测试 | 1h |

**总计**：6-9 小时到 working PoC。

## 与现有 Crate 嘅关系

```
big-head-loop  ──→  big-head-view (新)
  - ToolExecutor::render_view()   -  HTTP server
  - CycleResult.agent_responses   -  WebSocket
  - StateHook events              -  Static files
                                   -  React + R3F 前端
```

`big-head-view` 唔依赖 `big-head-loop` 嘅内部状态，只通过：
1. **Tool call**：agent 调 `render_view` → ViewCommand
2. **StateHook events**：订阅 `AgentActivity` 变化（read-only）

保持单向依赖，唔会循环。

## 未来扩展

- **Panels 关系**：agent 显式 emit "table X 喺 chart Y 旁边"
- **Panel 联动**：点击 table row → highlight chart 对应点
- **Spatial memory**：agent 记住 user 视点，下次去返去同一处
- **Voice control**：Web Speech API + "打开 tickaction 状态"
- **Persistence**：3D 空间布局存落 checkpoint
- **Multi-renderer**：同一 protocol 推到多个 renderer（远程 client、多 user 协作）
- **Replaceable backends**：未来可加 Tauri / Bevy renderer，agent 唔需要改

## 生态参考与协议决策

> 决策日: 2026-06-03. 在动手 `big-head-view` 之前, 扫咗 6 个相邻生态.

### 6 个生态速览

| 生态 | 协议 | 传输 | UI 模型 | License | 备注 |
|------|------|------|---------|---------|------|
| **[A2UI](https://a2ui.org/)** (Google + CopilotKit) | 声明式 UI (catalog + components) | A2A / AG-UI / WS / SSE | adjacency list, JSON Pointer 数据绑定 | Apache 2.0 | v0.8 stable / v0.9 current, 有官方 React 渲染器 |
| **[AG-UI](https://docs.ag-ui.com/)** (ag-ui-protocol) | 事件流 (16 种事件) | SSE / WS / HTTP | 事件类型系统, 跟 A2UI 互补 | MIT | RunStarted/Finished, TextMessage, ToolCall, StateDelta |
| **[OpenAI ChatKit](https://openai.github.io/chatkit-python/)** | 隐式 (SSE 事件流) | SSE | `.widget` 文件 + DynamicWidgetRoot, JSX-like dialect | Apache 2.0 | beta (`chatkit_beta=v1`), 强耦合 OpenAI Agents SDK |
| **[Assistant UI](https://github.com/Yonom/assistant-ui)** (YC) | - (UI 库) | - | React primitives | MIT | 提供 `@assistant-ui/react-ag-ui` + `@assistant-ui/react-a2a` 一等包 |
| **[LiveKit Agent Playground](https://github.com/livekit/agents-playground)** | - (PoC) | WebRTC | 视频/音频/chat UI | Apache 2.0 | WIP, 聚焦实时多模态 |
| **[agent-ui](https://github.com/agno-agi/agent-ui)** (Agno) | REST + Bearer | HTTPS | Next.js + Tailwind + shadcn | MIT | 接 AgentOS, tool 可视化 |
| **[Google CX / Dialogflow](https://cloud.google.com/dialogflow/cx/docs)** | custom payload + rich response | webhook JSON | 富响应: card, image, suggestion, custom payload | 闭源 | 文档不公开, Google bot wall 拦截 |

### 关键发现: A2UI vs AG-UI 嘅分工

呢两个经常被混淆, 但实际上**互补**:

- **A2UI** 系 *declarative UI description language*. Agent 唔会发事件, 而系发一坨 UI 定义 (类似 React component tree 嘅 JSON 表达), 由 client 渲染. 4 种核心 message: `createSurface`, `updateComponents`, `updateDataModel`, `deleteSurface`.
- **AG-UI** 系 *event transport protocol*. 16 种事件分 7 类: Lifecycle, Text Message, Tool Call, State Management, Activity, Special, Draft. 适合 streaming 场景 (RunStarted → TextMessageContent delta → ... → RunFinished).

A2UI 嘅 React 渲染器 (官方 `@a2ui/react`) **底层就系用 AG-UI 作为 transport**. 即: 声明式 UI 描述 + 事件流驱动更新.

### 决策: 我哋需要边样?

| 候选 | 利 | 弊 |
|------|----|----|
| **自研 `ViewCommand` enum** (原方案) | 完全 control, 3D position 字段天然契合, 冇 vendor lock | 重新发明轮子, assistant-ui 唔会支持, 生态隔离 |
| **A2UI v0.9 + AG-UI 传输** | 标准协议, 11+ 渲染器, assistant-ui 一等公民, 文档完整 | 3D position 唔系 A2UI 原生概念, 要扩展 catalog |
| **ChatKit SSE 模型** | widget 模板化好, OpenAI 主推 | beta, 强耦合 OpenAI ecosystem, 唔支持 3D |
| **MCP-UI** | Anthropic 官方, 跟 MCP 协议对齐 | 主要系 iframe + HTML, 唔适合 3D scene |

**结论**: 3D spatial canvas 系我哋嘅差异化. A2UI/AG-UI 嘅 catalog + adjacency list 模型好, 但**作为 3D scene 嘅"flat panel descriptor"唔够用**. 折中:

- **协议层**: 保留自研 `ViewCommand` enum (因为 position / rotation / scale 系一等公民)
- **UI 内容层**: A2UI v0.9 嘅 *adjacency list* 模型作为 panel `content` 嘅 schema
- **传输**: WebSocket JSON lines (同 A2UI 提议嘅 transport 一致, 未来可加 SSE)
- **未来兼容**: 跟 A2UI catalog 用同样嘅 JSON Schema 2020-12, 方便日后用 official React renderer 渲染 panel 内嵌 UI

具体: 一个 panel 嘅 `content` 可以系 `A2UIComponent[]` (adjacency list with `id` + `component` + props), 而 `position` / `rotation` 系我哋自己加嘅 spatial extension.

### A2UI v0.9 核心消息类型 (参考)

```jsonc
// agent → client, 4 种
{ "createSurface": { "surfaceId": "tickaction-status", "catalogId": "default" } }
{ "updateComponents": { "surfaceId": "tickaction-status", "components": [
  { "id": "root", "component": "Column", "children": ["header", "body"] },
  { "id": "header", "component": "Text", "text": "TickAction 状态" },
  { "id": "body",   "component": "Table", "columns": [...], "rows": [...] }
]}}
{ "updateDataModel": { "surfaceId": "...", "path": "/stats/count", "value": 42 } }
{ "deleteSurface":  { "surfaceId": "..." } }

// client → agent, 3 种
{ "action": { "name": "submit", "surfaceId": "...", "sourceComponentId": "btn-1",
              "timestamp": "2026-06-03T...", "context": { ... } } }
{ "error":  { "code": "VALIDATION_FAILED", "surfaceId": "...", "message": "..." } }
```

我哋嘅 `ViewCommand` 同 A2UI 嘅 1-to-1 映射:

| A2UI | big-head `ViewCommand` |
|------|------------------------|
| `createSurface` | `Spawn { id, kind, ... }` |
| `updateComponents` | `Update { id, content }` |
| `updateDataModel` | `Update { id, content: data_patch }` (kind 决定行为) |
| `deleteSurface` | `Despawn { id }` |
| client `action` | future `OnAction { id, payload }` callback |

> **重要**: 我哋唔系 A2UI 嘅 *strict* 客户端. 我哋 3D position 系 spatial extension. 标住呢个 deviation 之后, 写 doc 时要讲清楚边度偏离咗 A2UI 嘅 reference renderer, 同原因 (spatial canvas 系核心体验).

### ChatKit 嘅 widget 模式 (参考)

ChatKit 用 `.widget` 文件描述 widget template + data schema, server 端 hydrate → `DynamicWidgetRoot` Pydantic model → JSON 嵌入 `WidgetItem` → SSE stream. 呢个 *template + hydrate* 模式**值得抄**: 我哋嘅 `PanelKind::Table` 等可以定义 fixed template, LLM 只 fill data, 唔直接 emit 完整 layout, 减低 LLM 写错概率.

但 ChatKit 嘅强耦合 (SSE only, OpenAI agents SDK 依赖) 同 beta 状态令佢唔适合作为基础协议. 模式参考即可.

### Spatial 3D 嘅特殊性

A2UI / ChatKit / agent-ui 全部系 2D chat surface. **冇任何主流协议支持 spatial 3D position / rotation / scale**. 我哋嘅 3D 漂浮 panel 系 novel, 必须自研 protocol extension. 考虑过嘅几个方案:

1. **A2UI extension with spatial meta** (选呢个): 包 A2UI 嘅 adjacency list 入 `content`, 外面包 `position` / `rotation` / `scale`. 跟 A2UI 100% 兼容嘅 panel 内容, 但 spatial layout 喺最外层.
2. **全自研**: 完全独立, 唔参考 A2UI. 风险: 永远要维护自家协议.
3. **A2UI transport, custom body**: 用 AG-UI 事件流, 但 body 系自家 schema. 工作量大但最灵活.

折中 1 最务实: A2UI 跟住, spatial 创新自己落.

## 参考

### 协议 / 生态
- [A2UI 协议](https://a2ui.org/) - Google 嘅 declarative UI protocol
- [A2UI 概念总览](https://a2ui.org/concepts/overview/) - 4 种 message + 3 种 client payload
- [A2UI 组件](https://a2ui.org/concepts/components/) - adjacency list 模型
- [A2UI 数据绑定](https://a2ui.org/concepts/data-binding/) - JSON Pointer + templates
- [A2UI catalog](https://a2ui.org/concepts/catalogs/) - JSON Schema 2020-12
- [A2UI 传输](https://a2ui.org/concepts/transports/) - A2A / AG-UI / WS / SSE
- [AG-UI 事件规范](https://docs.ag-ui.com/concepts/events) - 16 种 event 类型
- [AG-UI 协议](https://github.com/ag-ui-protocol/ag-ui) - MIT
- [Assistant UI AG-UI 包](https://github.com/Yonom/assistant-ui) - 一等 A2UI + AG-UI 支持
- [OpenAI ChatKit](https://openai.github.io/chatkit-python/) - SSE + widget 模式 (Apache 2.0)
- [ChatKit 事件流](https://openai.github.io/chatkit-python/concepts/thread-stream-events/) - 11 种 event
- [ChatKit widgets](https://openai.github.io/chatkit-python/concepts/widgets/) - 3 种 root container
- [agent-ui (Agno)](https://github.com/agno-agi/agent-ui) - Next.js + Tailwind + shadcn
- [LiveKit agent-playground](https://github.com/livekit/agents-playground) - WebRTC 多模态
- [MCP-UI](https://github.com/anthropics/mcp-ui) - Anthropic 官方 UI 协议
- [OpenAI Apps SDK](https://openai.com/index/apps-sdk/) - AI 操控 UI
- [Anthropic Artifacts](https://docs.anthropic.com/en/docs/build-with-claude/artifacts) - 响应中嵌入可交互组件

### 渲染 / 库
- [R3F `<Html>`](https://github.com/pmndrs/drei#html) - 3D 空间漂浮 HTML (核心依赖)
- [a2ui-3d-renderer](https://github.com/josh-english-2k18/a2ui-3d-renderer) - 唯一已知嘅 A2UI 3D 渲染实验 (~2 ⭐)
- [Three.js](https://threejs.org/) - WebGL 底层
- [@react-three/fiber](https://r3f.docs.pmnd.rs/) - React 绑 Three.js
- [@react-three/drei](https://github.com/pmndrs/drei) - R3F helpers (OrbitControls, Html, etc.)

### 协议决策摘要

> **Spatial 3D 系 novel**, 冇主流协议支持. 我哋采 hybrid:
> - **协议**: 自研 `ViewCommand` enum (因为 position/rotation/scale 系一等公民)
> - **UI 内容 schema**: 参考 A2UI v0.9 嘅 adjacency list, 跟 A2UI catalog (JSON Schema 2020-12) 方便日后 official 渲染器兼容
> - **传输**: WebSocket JSON lines (未来可加 SSE, 跟 A2UI 提议嘅 transport 对齐)
> - **ChatKit 模式借鉴**: 复杂 widget (Table/Chart) 用 *template + LLM-fill-data* 模式, 减少 LLM 写错 layout 概率
> - **偏离点**: 3D spatial layout 喺最外层, 唔系 A2UI 标准 (A2UI 假设 2D chat surface)
