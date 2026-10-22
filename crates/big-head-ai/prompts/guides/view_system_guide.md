# View System（ECS display contract）

View 不是最后随手生成 HTML。View 是 ECS 世界的可读投影层：先判断用户需要看哪类状态，再选择固定 display slot，最后才调用 `render_view` 或 `render_status`。

如果用户在讨论 3030 / view server / 组件理解 / runtime inspector / TickAction 可视化，先读本指南，再决定怎么渲染。

## 固定区域

3030 浏览器以顶部 Tab 按用途分区，每次只挂载当前页面：

| 区域 | 固定用途 | 推荐内容 |
|---|---|---|
| `mind-view` | 独立 Mind View Tab | Canvas / WebGL / Three.js / 概念动态图 |
| 每个 `html2d` panel | 独立 panel Tab | TickAction、Spark、SessionContext、WorkItem、表格、表单、选择项 |
| `spatial3d` | Spatial World Tab | 短节点、关系卡片、空间 cluster，不放长文 |
| `render_status` | 桌面/菜单栏轻状态 | 当前任务、进度、阻塞、用户选择 |

不要把所有东西塞进 `mind-view`。`mind-view` 只负责“看见结构/场/动态”，可读解释和操作按钮应该放普通 `html2d` panel。
写 `mind-view` 时直接调用 `render_mind_view({title, markup, runtime})`；它会补齐稳定 panel contract 并覆盖更新，
不要先查询 ViewState，也不要手拼通用 `render_view` update envelope。

## Display Intent

渲染前先选一个 intent：

| intent | 触发场景 | 输出 |
|---|---|---|
| `inspect_tickactions` | 用户问 tick/action/自主循环/为什么一直跑 | `html2d` inspector，固定 id `inspector-tickactions` |
| `inspect_sparks` | 用户问 Spark 队列、积压、是否消费 | `html2d` inspector，固定 id `inspector-sparks` |
| `inspect_context` | 用户问时间、天气、地点、ambient cue | `html2d` inspector，固定 id `inspector-context` |
| `inspect_world` | 用户问 ECS 世界里有什么 | `html2d` inspector，固定 id `inspector-world` |
| `visualize_relation` | 用户要图形、关系、动态理解 | 更新 `mind-view`，可选再补 info board |
| `choose_next` | 需要用户选择 A/B/C | `render_status` choice 或 `html2d` form/a2ui |
| `debug_view` | CSS 重叠、viewport、canvas 尺寸、3D panel 乱 | `html2d` diagnostic table，不要只改视觉 |

调用 `render_view` 时可以直接传 `intent`，handler 会从 Rust display registry 自动补齐
`id/surface/kind/title/position`。ECS inspector 不应再手填这些字段。

查看单个 ECS 组件时优先调用：

```json
{"component":"TickAction","limit":20}
```

工具：`inspect_component_view`。它在 runtime 内一次完成实体枚举、Rust/动态组件读取、状态推导、
字段裁剪、稳定 panel 选择、render 和 `ViewState` 持久化。不要再用多轮
`get_world_overview → query_entities → get_entity → 手工 render_view` 拼同一份 Inspector。
只有组合多个组件、定制关系图或交互布局时才直接调用 `render_view`。

## ECS 组件到 View 的映射

| ECS 内容 | 应该怎么显示 | 不要怎么做 |
|---|---|---|
| `TickAction` | 表格：eid、cooldown、last_run、remaining、script head、last error | 不要在 3D panel 里放完整脚本 |
| `Spark` | 表格：eid、status、timestamp、text head、result head | 不要靠 `get_world_overview` 反复扫 |
| `SessionContext` | card + small table：location、time_of_day、light、weather、source | 不要把天气 trail 无限 append |
| `AmbientSignal` | active/expired summary + 最近 N 条 | 不要只显示 total，不区分 live |
| `AgentState` | card：wave、total_actions、max_iterations、mode | 不要把 wave drift 当成用户任务 |
| `ToolHistory` | 最近工具调用列表，失败突出 | 不要把大段 tool output 塞进 prompt |
| `ShardData` | 索引表：paradigm、count、preview、如何取回 | 不要直接展开所有 shard |
| `ViewState` | panel inventory：id、surface、kind、title | 不要 `modify_component` 改 ViewState，用 `render_view` |

## 固定 Panel ID

使用稳定 id 覆盖更新，避免每次新建 panel：

```text
mind-view
inspector-tickactions
inspector-sparks
inspector-context
inspector-world
inspector-tools
inspector-view
decision-board
work-board
```

更新已存在 panel 时优先：

```json
{
  "op": "update",
  "intent": "inspect_tickactions",
  "content": {
    "kind": "inspector",
    "component": "TickAction",
    "summary": [{"label":"active","value":0,"tone":"neutral"}],
    "entries": []
  }
}
```

如果不确定 panel 是否存在，可以直接 `spawn` 同 id；view server 会按当前实现覆盖/同步 snapshot。

## TickAction Inspector 最小形状

用户问“现在为什么一直 tick / 哪个 action 会触发 / CD 是多少”时：

1. `inspect_component_view({"component":"TickAction","limit":20})`
2. 如用户确实追问环境或 agent 状态，再分别 inspect `SessionContext` / `AgentState`
3. 最终回复只总结异常和下一步，不要继续反复查

每个 `entries[]` 使用以下字段：

```text
entity_id | label(role) | status | fields[cooldown,last_run,since,remaining,script_head]
```

`role` 由 runtime 从 script head 推断：`ambient_heartbeat`、`refresh_session_context`、`weather_query`、`reflect_forward` 等。

## View Debug 最小形状

用户说“3030 看起来遮住/重叠/只看到左上角/没图形效果”时，不要只口头解释。先生成一个 diagnostic panel：

```text
id: inspector-view
surface: html2d
kind: table/html
内容：panel id、surface、kind、raw runtime、expected slot、observed issue、next fix
```

如果问题是 mind-view 的 Canvas/WebGL 尺寸：

- raw runtime 必须加 `data-big-head-mind-runtime="raw"`。
- canvas CSS 尺寸用 `clientWidth/clientHeight` 或 `getBoundingClientRect()`。
- backing buffer 用 `cssW * dpr` / `cssH * dpr`。
- layout、camera、projection、text placement 一律用 CSS px，不要用 backing buffer px。
- scale 从数据包围盒自适应计算，不要硬编码。

## 交互调度

显示层有两条交互路径：

| 路径 | 用途 |
|---|---|
| `handler: {type:"rhai", tool_id:"..."}` | 已知、确定、低风险动作，点击后直接执行 |
| 默认 view event Spark | 需要 agent 理解上下文、解释、拆任务 |

能确定的 UI 操作尽量走 Rhai fast lane：打开详情、切换 tab、标记 Done、运行已有 system。需要判断用户意图时才走 Spark。

## 完成条件

如果用户要求“优化 view / 展示状态 / 看看组件 / 做 inspector”，完成至少需要满足一项：

- 调用 `render_view` 更新一个固定 panel。
- 调用 `render_status` 更新桌面状态。
- 明确说明“这次只设计 contract，尚未渲染”。

不要把“我会渲染”当作已经渲染。
