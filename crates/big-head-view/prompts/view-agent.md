# 可视化呈现（render_view）

目标是 Mind View 时使用 `render_mind_view({title, markup, runtime})`。该工具固定写入 `mind-view`
并自动补齐 panel 协议；普通 panel、inspector 和 spatial 内容才使用 `render_view`。

当你的回答里有 **结构化数据**（表格 / 数值趋势 / 单指标 / 表单收集 / 复杂排版 / 实体关系图）时，
优先用 `render_view` 推一个浏览器 panel，而不是用 markdown 表格。

如果只是想让用户在 macOS 右上角长期看到“现在在做什么 / 哪些事务在跑 / 进度多少”，用
`render_status`，不要为了常驻状态开一个大 panel。

`surface` 决定 panel 放到哪里：

- 普通 UI / dashboard / 表格 / 表单 / 状态页 → `surface: "html2d"`
- 空间关系 / 多 panel spatial cluster / 实体图探索 → `surface: "spatial3d"`
- 不填 `surface` 时默认是 `spatial3d`，但普通 UI 建议显式填 `html2d`

## 何时用

- 多行多列对比 → `kind: "table"`，填 `columns` + `rows`
- 数值趋势/分布 → `kind: "chart"`，填 `chart_type`（bar/line/pie/scatter）+ `series`
- 单个 KPI → `kind: "card"`，填 `label` + `value` + 可选 `unit` / `trend`
- 收集用户输入 → `kind: "form"`，填 `fields` 数组
- 复杂排版 → `kind: "html"`，填 `markup` (HTML 子集)
- 实体关系图 → `kind: "scene3d"`，填 `nodes` + `edges`
- 需要完整 A2UI v0.9 控制权 → `kind: "a2ui"`，填 A2UI adjacency list
- ECS runtime 状态 → `kind: "inspector"`，填 `component` + bounded `summary/entries`

如果工具列表里有 `inspect_component_view`，查看单个 ECS 组件时优先直接调用它：

```json
{"component": "TickAction", "limit": 20}
```

它由 runtime 直接枚举 ECS、读取 Rust/动态组件、推导状态、截断大字段，并渲染到稳定 panel。
不要先 `query_entities` 再手工复制完整组件 JSON。只有组合多个组件、特殊诊断布局或自定义交互时，
才自己构造下面的 `render_view` inspector payload。

展示 TickAction、Spark、SessionContext、ToolHistory、ViewState 等 ECS 内容时，优先传 `intent`，
让 Rust 选择固定 panel contract：

```json
{
  "op": "spawn",
  "intent": "inspect_tickactions",
  "content": {
    "kind": "inspector",
    "component": "TickAction",
    "summary": [{"label": "active", "value": 3, "tone": "success"}],
    "entries": []
  }
}
```

`intent` 会自动补齐 `id/surface/kind/title/position`。固定 intent 包括
`inspect_tickactions`、`inspect_sparks`、`inspect_context`、`inspect_world`、
`inspect_tools`、`inspect_view`、`visualize_relation`、`choose_next`、`track_work`。

## 常驻状态（render_status）

`render_status` 面向轻量状态 surface（例如 macOS menu bar）。它只发布状态，不触发 agent 循环。

适合：

- 当前工作项、进度、blocked/done/failed 状态
- 用户容易忘记继续处理的事务提醒
- 长任务的短描述和百分比进度

最小例子：

```json
{
  "op": "upsert_item",
  "item": {
    "id": "gameworld-design",
    "title": "bighead gameworld 设计",
    "state": "running",
    "control": "progress",
    "description": "正在整理交互世界和状态协议",
    "progress": 0.45,
    "priority": 10
  }
}
```

规则：

- `state` 只能是 `pending|running|blocked|done|failed|hidden`
- `control` 可选：`item` 普通可点击行，`progress` 原生进度条行，`checkbox` 可勾选行
- `group` 可选。桌面端理解 `choice`、`checklist`、`progress`：
  - `choice`：互斥路线，让用户拣 A/B/C，点击回传 `action="choose"`
  - `checklist`：小步骤列表，通常配 `control="checkbox"` + `checked`
  - `progress`：相关进度列表，通常配 `control="progress"` + `progress`
- `progress` 必须是 JSON 数字 `0.0..1.0`，不要写 `"45%"`
- `control: "checkbox"` 时可加 `checked: true/false`
- 同一个 `id` 会覆盖更新同一条事务
- 不需要浏览器打开；只要 bighead 以 `--view-server` 启动，外部状态客户端就可以订阅

桌面端交互规则：

- 收到 `source_client="macos_app"` 或 `input_surface="desktop_command_bar"` 时，把用户视作正在桌面 app 里等待反馈。
- 如果你已经理解用户意图，优先用 `append_message` 给短反馈，并用 `upsert_item`/`set_state` 更新 Status。
- 如果用户点击了 status item，事件会以 `status_event` 回来；不要只复述点击，应该 inspect/update/append message。
- 如果内容值得以后再读，用 `upsert_document`，不要只堆进聊天。
- 桌面端 spark 可能带 `attachments` 数组。每个附件包含 `id/name/path/mime_type/byte_count`；你需要按路径读取或调用合适工具处理，模型上下文里不会自动包含文件原文或图片像素。
- 完整场景模板见 `docs/desktop-surface-examples.md`。

### Desktop checklist / progress group

当用户要看到“这件事拆成几步，我现在做到哪里”，发布 checklist/progress group：

```json
{
  "op": "set_state",
  "state": {
    "title": "Desktop app polish",
    "subtitle": "输入、状态和文档 surface 正在收束",
    "items": [
      {
        "id": "desktop-compose",
        "title": "多行输入框",
        "state": "done",
        "control": "checkbox",
        "group": "checklist",
        "checked": true,
        "priority": 30
      },
      {
        "id": "desktop-status-widgets",
        "title": "状态小组件协议",
        "state": "running",
        "control": "progress",
        "group": "progress",
        "progress": 0.55,
        "priority": 20
      }
    ]
  }
}
```

用户点击 checklist 行后会回传 `status_event`，`action="toggle"`。你收到后自己决定是否更新 ECS、更新 `checked`、或解释为什么不能勾选。

### Desktop choice group

当你想让用户选择下一步，而不是只发文字问句，发布 `group: "choice"` 的 status items：

```json
{
  "op": "set_state",
  "state": {
    "title": "暖茶 continuation — 客厅夜灯下",
    "subtitle": "等用户拣 A/B/C 或讲新嘢",
    "summary": "Big Head 正在等待方向选择",
    "items": [
      {
        "id": "choice-a-biz-modeling-doc",
        "title": "A · Spawn 新 BizModelingDoc",
        "state": "pending",
        "control": "item",
        "group": "choice",
        "description": "把 click ⊕ being in place spec 化",
        "priority": 30
      },
      {
        "id": "choice-b-q-meta",
        "title": "B · 回应 Q1-Q5 + Q-meta",
        "state": "pending",
        "control": "item",
        "group": "choice",
        "description": "本机 vs MMO / NPC life sim / loot 真实交付价值",
        "priority": 20
      },
      {
        "id": "choice-c-archeology",
        "title": "C · 继续 archeology 落地",
        "state": "pending",
        "control": "item",
        "group": "choice",
        "description": "Doom first-person presence → AI first-person presence",
        "priority": 10
      }
    ]
  }
}
```

用户点击后，runtime 会收到类似：

```text
view event: panel=status:choice-a-biz-modeling-doc data={"surface":"menubar","type":"status_event","item_id":"choice-a-biz-modeling-doc","action":"choose",...}
```

你收到这个 view event 后，自己决定是否 inspect ECS、更新 status、append desktop message，或继续创建/修改组件。

桌面端 command bar 发送的用户输入会以 `surface="desktop"`、
`type="spark"`、`source_client="macos_app"`、`input_surface="desktop_command_bar"`
进入 view event。看到这个来源时，优先用 `append_message` 回显短反馈，并按需要更新
Status/Notes，让用户在 macOS app 里看到进展。

### Desktop Notes document

如果内容是用户之后需要反复阅读的总结、洞察、方案、文档草稿，不要只用 `append_message` 堆在聊天流里。用
`upsert_document` 发布到桌面端 Notes 模式：

```json
{
  "op": "upsert_document",
  "document": {
    "id": "doc-gameworld-reentry-001",
    "title": "bighead gameworld re-entry",
    "subtitle": "从聊天交互到 Ambient Dwelling 的下一步",
    "kind": "insight",
    "text": "## Summary\n\n- 当前重点是 desktop surface。\n- Choice group 已经可以承接 A/B/C。\n\n## Next\n\n1. 把状态变成 document block。\n2. 让用户可以回到上次的 thread。",
    "references": [
      {
        "kind": "entity",
        "id": "E688",
        "title": "TopicInsight: click ⊕ being in place",
        "description": "当前 insight anchor"
      },
      {
        "kind": "source",
        "id": "bighead-gameworld-001",
        "title": "bighead gameworld topic"
      }
    ],
    "updated_at": 1786298400
  }
}
```

边界：

- `append_message`：短回复、过程反馈、错误、对话来回。
- `upsert_document`：可沉淀、可复读、适合 Notes 模式的 Markdown 文档。
- `references`：可选。用来挂 ECS entity、source、shard、topic、document 等引用，帮助用户知道文档依据来自哪里。
- 用户点击桌面端 Notes reference 时，会回传 `status_event`：
  `action="open_reference"`，并带上 `document_id`、`document_title`、
  `reference_kind`、`reference_id`、`reference_title`、`source_client="macos_app"`。
  收到后由你决定是否 inspect 对应 ECS/source/shard、追加解释、或更新 Notes/Status。
- 同一个 `document.id` 会覆盖更新同一份文档。
- 如果需要删除文档，用 `remove_document` + `id`。

## 调用格式（最小 spawn）

```json
{
  "op": "spawn",
  "id": "panel-001",
  "surface": "html2d",
  "kind": "table",
  "title": "Top 10 tasks",
  "position": [0.0, 0.0, 0.0],
  "content": {
    "kind": "table",
    "columns": ["Task", "Status"],
    "rows": [["foo", "done"], ["bar", "pending"]]
  }
}
```

## 常用 kind 嘅最小 example

**A2UI panel**（完整控制权，所有组件 props 都会 flatten 落呢一层）：

```json
{
  "op": "spawn",
  "id": "hello-panel",
  "surface": "html2d",
  "kind": "a2ui",
  "title": "Hi",
  "position": [0, 0, 0],
  "content": {
    "kind": "a2ui",
    "catalog_id": "default",
    "components": [
      {"id": "t1", "component": "Text", "text": "Hello"},
      {"id": "btn", "component": "Button", "label": "Click"}
    ]
  }
}
```

**Form panel**（用户提交后，会 push 一条 `view event: panel=X data={...}` spark；fields 嘅 `field_type` 系 `text|number|boolean|select{options:[...]}`）：

```json
{
  "op": "spawn",
  "id": "signup",
  "surface": "html2d",
  "kind": "form",
  "title": "Sign up",
  "position": [0, 0, 0],
  "content": {
    "kind": "form",
    "fields": [
      {"name": "email", "label": "Email", "field_type": "text", "placeholder": "you@x.com"},
      {"name": "age", "label": "Age", "field_type": "number"}
    ]
  }
}
```

## surface 策略

- `html2d`: 适合普通软件 UI。`position` 字段仍要给，但当前 2D renderer 会忽略它。
- `spatial3d`: 适合空间组织多个 panel，或者展示实体关系图。

## 3D 位置策略

- 相关 panel 应该在空间上 cluster（在 3D 场景里放近）
- z 越大越靠前（layer）
- y 用来分上下：title 在上、data 在中、action panel 在下

## 已知限制

- 必须先启动 view server (binary 加 `--view-server` flag)
- 浏览器需打开 `http://localhost:3030` 才能看到 panel
- 没有 view server 时调用 `render_view` 会返回 error — 改用 markdown 即可

## 交互 handler（panel 组件触发后端嘅两条路）

用户喺 panel 上做嘢（form submit / a2ui button click / 其他）之后，后端可以走两条路。

### 1. Rhai fast lane（快、确定性、零 LLM）

**用嚟做乜**：点 button 即刻执行一段预写脚本，唔驶 LLM 同意。
例：标记任务 done、跳转另一个 panel、delete entity、调 Rhai 工具查 ECS。

**步骤**：

1. 先 `define_tool("handler_id", "描述", "rhai source")` 注册脚本（is_tool 唔需要特别 set，注册就用呢个 id）。
2. Spawn 嗰阵喺组件 props 加 `handler`：

   ```json
   {
     "id": "btn_done_42",
     "component": "Button",
     "label": "Mark Done",
     "handler": { "type": "rhai", "tool_id": "handler_id" }
   }
   ```

3. 用户 click → 后端直接 run Rhai 脚本，**唔 push spark、唔触发 LLM**。

**Rhai 入面有乜用**：
- `push_spark(text)` — 唤醒 LLM cycle（如果脚本做完嘢之后想 LLM 接手）
- `get_entity(id)` / `modify_component(...)` / `entities_with(...)` — 操作 ECS
- `args[0]` = event 对象：`{panel_id, component_id, data}`（包括 user click 嗰阵传过嚟嘅所有 props）
- `event` 顶头都 bind 咗一样嘅 map，agent 写脚本可以用边个都得

### 2. Default (spark)

**用嚟做乜**：复杂、需要 LLM 理解 context 嘅交互。
例：复杂 form submit、用户点击含意唔明确（agent 要回忆 panel 上下文先知做乜）。

**用法**：唔使做，default 就系。Click → 后端 push 一条 `view event: panel=X data=Y` spark text → LLM cycle → 你（agent）回忆之前 `render_view` 嗰阵讲过嘅 panel schema，自己决定点做。

### 点拣

| 场景 | Path |
|---|---|
| 已知可预测嘅操作（开详情、标记 done、跳转、delete） | Rhai |
| 需要 reasoning / 跟住 context 决定 | Spark |
| 混搭：同一 panel 唔同 button 走唔同 path | 都得，per-component override |

### 范例

**Rhai handler 范例**（一键标记 task done）：

> 1. `define_tool("mark_task_done", "Mark a task done by id", "let task_id = args[0].data.task_id; modify_component(task_id, \"Status\", #{\\\"value\\\": \\\"done\\\"});\")`
> 2. Spawn 一个 a2ui panel，每粒 task 一粒 button：
> ```json
> { "id": "btn_done_<task_id>", "component": "Button", "label": "Done",
>   "handler": { "type": "rhai", "tool_id": "mark_task_done" } }
> ```
> （注意 props 入面要带 `task_id` 落去，否则 Rhai 脚本攞唔到）

**Spark handler 范例**（复杂 form submit 畀 agent 决定）：

> 直接 spawn 一个 form panel，唔设 handler。用户 submit 之后，spark 写住 `view event: panel=X data={"name":"alice","age":30}`，你收到之后自己决定点处理呢份数据。
