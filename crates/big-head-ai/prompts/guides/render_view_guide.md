# 可视化呈现（render_view）

如果目标明确是 `mind-view`，优先调用专用工具：

```json
{"title":"关系场","markup":"<!doctype html>...","runtime":"raw"}
```

工具名：`render_mind_view`。它会固定使用 `mind-view/html2d/html` 并执行 upsert，自动补齐 raw runtime
标记；不要先查询 panel 是否存在，也不要再手写 `op/id/surface/kind/position/content/patch` 外壳。
只有普通 panel、inspector、table、form 或 spatial3d 内容才使用通用 `render_view`。

当你的回答里有 **结构化数据**（表格 / 数值趋势 / 单指标 / 表单收集 / 复杂排版 / 实体关系图）时，
优先用 `render_view` 推一个浏览器 panel，而不是用 markdown 表格。

如果你是在展示 ECS 世界本身（TickAction / Spark / SessionContext / AmbientSignal / ToolHistory /
ViewState / runtime inspector），先运行 `run_system("guide_view_system")`，按固定 display intent
和 panel id 选择位置，再调用 `render_view`。

如果只是想让用户在 macOS 右上角长期看到“现在在做什么 / 哪些事务在跑 / 进度多少”，用
`render_status`，不要为了常驻状态开一个大 panel。

## 最后一跳必须执行

口头描述一个画面、写出城市设计概念、说“我会更新 mind-view”都不会改变浏览器。

如果你向用户承诺已经更新或将要更新 3030 / browser view / mind-view，必须在最终回复前实际调用
`render_view` 或 `render_mind_view`。最终回复之前应该能在日志里看到类似：

```text
tool render_view({...})
OK render_view: ...
```

如果没有调用 `render_view`，就只能说“我设计了方案，但还没有渲染到 view”。不要把计划说成完成。

`surface` 决定 panel 放到哪里：

- 普通 UI / dashboard / 表格 / 表单 / 状态页 → `surface: "html2d"`
- 空间关系 / 多 panel spatial cluster / 实体图探索 → `surface: "spatial3d"`
- 可编程的 2D WebGL / Three.js 画布 → `id: "mind-view"` + `surface: "html2d"` + `kind: "html"`
- 不填 `surface` 时默认是 `spatial3d`，但普通 UI 建议显式填 `html2d`

## Mind View（2D programmable WebGL/HTML slot）

3030 浏览器现在使用顶部 Tab，每个 panel 是一个独立页面：

- `Mind View` Tab：sandboxed full-page visual runtime，适合 Canvas、WebGL、Three.js、shader demo、概念动态图。
- 每个普通 `html2d` panel：各自拥有一个 Tab，适合 table/card/form/chart/a2ui 和 runtime inspector。
- `Spatial World` Tab：集中显示 3D floating panel / world scene。

阅读性规则：

- 长文档、markdown 说明、表格、状态清单、用户要认真读的内容，优先放 `surface: "html2d"`。
- `spatial3d` 只放短卡片、关系节点、少量按钮或概念锚点；不要把大段 prose 塞进 3D panel。
- 动态视觉 / shader / Three.js / WebGL 场景优先更新 `id: "mind-view"`，不要在 3D floating panel 里塞完整画布。
- 如果使用 `id: "mind-view"`，内容必须有明确视觉程序：`<canvas>` + WebGL/shader，或 Three.js 场景，或至少 CSS/Canvas 动态效果。不要只发送静态文字 HTML。
- 所有 2D/3D 可视化都必须避免文字、按钮、图表互相重叠。用 grid/flex/normal document flow；不要用随意的 `position:absolute`/负 margin/大 transform 做排版。
- 默认情况下，mind-view 只运行 Big Head 内置的 visual runtime，并把你提供的普通 HTML 作为安全 readout 显示，剥离 `<script>/<style>/canvas/svg`，避免重叠。
- 只有当你确实写了完整、自洽、无重叠的视觉 runtime，才在 `<html>` 或 `<body>` 加 `data-big-head-mind-runtime="raw"`。raw runtime 会占满整个 mind-view body，不再显示 readable split；长文档必须放普通 `html2d` 信息板。
- raw runtime 的 Canvas/WebGL/Three.js 必须基于 iframe 实际 CSS 尺寸自适应：用 `container.clientWidth/clientHeight` 或 `getBoundingClientRect()` 计算 CSS 宽高，再乘 `devicePixelRatio` 设置 canvas buffer。注意 CSS 尺寸和 backing buffer 尺寸必须分开：如果 `canvas.width = cssW * dpr` 且 `ctx.setTransform(dpr, 0, 0, dpr, 0, 0)`，后续所有布局、投影 origin、`fillRect`、坐标计算都必须使用 `cssW/cssH`，不能继续使用 `canvas.width/canvas.height`，否则会被 DPR 放大并偏到画面外。
- 投影 scale 必须由当前 CSS 宽高和数据包围盒计算，例如 `scale = min(cssW * 0.8 / projectedWidth, cssH * 0.7 / projectedHeight)`；禁止硬编码 `asc=12`、固定 camera scale 或固定 pixel extent。

当用户要你“画一个思维空间 / 做一个动态可视化 / 用 WebGL 展现关系 / 改一下画面”时，
直接调用 `render_mind_view`：

```json
{
  "title": "mind-view · relationship field",
  "runtime": "raw",
  "markup": "<!doctype html><html><head><meta charset=\"utf-8\"><style>html,body{margin:0;height:100%;overflow:hidden;background:#05070d}canvas{width:100vw;height:100vh;display:block}</style></head><body><canvas id=\"c\"></canvas><script type=\"module\">const canvas=document.getElementById('c');const gl=canvas.getContext('webgl');function resize(){const dpr=Math.min(devicePixelRatio||1,2);canvas.width=innerWidth*dpr;canvas.height=innerHeight*dpr;gl.viewport(0,0,canvas.width,canvas.height)}addEventListener('resize',resize);resize();let t=0;function frame(){t+=0.016;gl.clearColor(0.03+Math.sin(t)*0.02,0.05,0.12,1);gl.clear(gl.COLOR_BUFFER_BIT);requestAnimationFrame(frame)}frame();</script></body></html>"
}
```

每次调用都会覆盖同一个稳定 panel，不需要区分 spawn/update。

Mind View 规则：

- 如果要自定义 WebGL/Three.js，`markup` 应该是完整 HTML 文档，包含自己的 `<style>` 和 `<script type="module">`；`runtime: "raw"` 会自动加运行标记。
- raw runtime 的脚本解析错误和未处理 Promise 错误会回传为 `view event: panel=mind-view data={"type":"mind_runtime_error","message":"...","line":28,"column":...}`。收到后按位置修正 markup，并再次调用 `render_mind_view`。工具成功只代表发布和持久化成功，浏览器执行结果由该异步事件确认。
- 如果使用 Three.js，可以从 CDN import，例如 `https://cdn.jsdelivr.net/npm/three@0.169.0/build/three.module.js`。
- iframe 默认会运行脚本，但不能直接读父页面状态；需要交互时先用普通 form/a2ui panel，或之后扩展 view_event bridge。
- 不要把普通长文档塞进 `mind-view`；长文档用左下信息板或 desktop Notes。
- 如果只想发布文本、表格、选择项、状态卡，不要用 raw mind runtime；用普通 `surface: "html2d"` panel。

## 何时用

- 多行多列对比 → `kind: "table"`，填 `columns` + `rows`
- 数值趋势/分布 → `kind: "chart"`，填 `chart_type`（bar/line/pie/scatter）+ `series`
- 单个 KPI → `kind: "card"`，填 `label` + `value` + 可选 `unit` / `trend`
- 收集用户输入 → `kind: "form"`，填 `fields` 数组
- 复杂排版 → `kind: "html"`，填 `markup` (HTML 子集)
- 实体关系图 → `kind: "scene3d"`，填 `nodes` + `edges`
- 需要完整 A2UI v0.9 控制权 → `kind: "a2ui"`，填 A2UI adjacency list

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
- `html2d` + `id: "mind-view"` + `kind: "html"`: 适合左上可编程 WebGL/Three.js 画布。
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

1. 先 `define_tool("handler_id", "描述", "rhai source")` 注册脚本。
2. Spawn 嗰阵喺组件 props 加 `handler`：

   ```json
   {
     "id": "btn_done_42",
     "component": "Button",
     "label": "Mark Done",
     "handler": { "type": "rhai", "tool_id": "handler_id" }
   }
   ```

3. 用户 click → 后端直接 run Rhai 脚本，唔 push spark、唔触发 LLM。

**写 handler 脚本前**，先 `run_system("guide_rhai_syntax")` 拉 Rhai 语法参考。

**event 对象**（`args[0]` / 顶层 `event` 变量）：

```
ev.panel_id      // "panel-001"
ev.component_id  // "btn_done_42"（a2ui button 嘅 id）
ev.data          // 用户传嘅数据（form values / button props 等）
```

### 2. Default (spark)

**用嚟做乜**：复杂、需要 LLM 理解 context 嘅交互。

**用法**：唔使做，default 就系。Click → 后端 push 一条 `view event: panel=X data=Y` spark text → LLM cycle → agent 自己决定点做。

### 点拣

| 场景 | Path |
|---|---|
| 已知可预测嘅操作（开详情、标记 done、跳转、delete） | Rhai |
| 需要 reasoning / 跟住 context 决定 | Spark |
| 混搭：同一 panel 唔同 button 走唔同 path | 都得，per-component override |

### 范例

**Rhai handler 范例**（一键标记 task done）：

> 1. `define_tool("mark_task_done", "Mark a task done by id", "let task_id = args[0].data.task_id; modify_component(task_id, \"Status\", #{\\\"value\\\": \\\"done\\\"});")`
> 2. Spawn 一个 a2ui panel，每粒 task 一粒 button：
> ```json
> { "id": "btn_done_<task_id>", "component": "Button", "label": "Done",
>   "handler": { "type": "rhai", "tool_id": "mark_task_done" } }
> ```

**Spark handler 范例**（复杂 form submit 畀 agent 决定）：

> 直接 spawn 一个 form panel，唔设 handler。用户 submit 之后，spark 写住 `view event: panel=X data={"name":"alice","age":30}`，你收到之后自己决定点处理。
