# Agent Modes（可选择的思维 / 执行通道）

Big Head 的默认循环是 ReAct / Tool Loop：收到 Spark，推理，调用工具，观察结果，再进入下一 wave。
当任务变复杂时，不要把所有事都塞进同一种 reasoning。你可以先选择一个 mode，并把 mode 显式落到 ECS、
status panel、Rhai system 或 TickAction 里，让世界知道你正在用哪条通道工作。

## 核心语义

Mode 不是临时口头风格，而是一个可被世界读取、可被用户看见、可被脚本触发的执行颗粒。

推荐链条：

```text
Spark / event / threshold
  -> 选择 AgentMode
  -> 如有必要，spawn/modify ECS mode record
  -> 如有必要，render_view / render_status 给用户看
  -> 简单任务直接 ReAct
  -> 长期/条件任务 define_system 或 TickAction
  -> 执行后用 Transaction/ActionRecord/WorkItem 留痕
```

不要为了 mode 新建过多核心组件。第一版优先复用现有能力：

- `define_component`：按需定义轻量 mode / plan / work item schema。
- `spawn_entity` / `modify_component`：把当前 mode、计划、状态、选择项写入 ECS。
- `define_system`：把可重复执行的步骤沉淀成 Rhai system。
- `TickAction`：把基于时间、状态、阈值的自动检查落成脚本化触发。
- `push_spark`：把长任务拆成后续 spark。
- `render_view` / `render_status`：把 mode、分支、进度、选择展示给用户。

## 建议的 mode

| mode | 用途 | 典型落地 |
|------|------|----------|
| `react` | 普通 Spark、短任务、直接工具调用 | 直接执行，少量 ECS mutation |
| `plan_execute` | 多步骤任务、代码开发、整理资料 | `Plan` / `WorkItem` / status progress |
| `supervisor` | 一个主 agent 分派多个专门子任务 | 多个 `WorkItem` + `push_spark` |
| `handoff` | 把任务移交给另一个 system / tool / world | `Lens` / bucket / `run_system` |
| `graph_workflow` | 有条件分支、并行、循环的流程 | ECS 节点/边 + Rhai system |
| `critic` | 检查失败、schema、UI 重叠、权限、输出质量 | 非破坏性 query + 修复建议 |
| `approval` | 需要用户确认的删除、发布、跨 world 交换 | `render_status` choice / form |
| `ambient` | 时间、地点、天气、quietness、背景感知 | `AmbientSignal` + `TickAction` |
| `projection` | 图片/OCR/source/shard/lens/事实落 ECS | `ShardData` / source index / relation |
| `render` | 用户需要看见状态、图、表、控制面板 | `render_view` / `render_status` |
| `sandbox` | 先模拟 diff，不立刻写世界 | query + plan + 用户确认后 mutation |

## 选择规则

- 用户一句普通请求，几步内可完成：用 `react`。
- 任务明显超过一次回答、需要阶段拆分：用 `plan_execute`，并考虑 `push_spark`。
- 同时有多个方向、用户需要选择：用 `approval` + `render_status` choice。
- 工具失败、schema validation、UI overlap、HTML 覆盖、数据类型错误：切到 `critic`，先 inspect 再修。
- 遇到大内容、图片、OCR、网页 source、lens、跨 world 数据：用 `projection`，把事实和索引落 ECS。
- 需要长期自动观察：用 `ambient` 或 `TickAction`。TickAction 不能直接调用 LLM，只能 spawn Spark 或写状态。
- 需要给用户视觉反馈：用 `render`，优先选择合适 surface；复杂视觉用 `mind-view`，可读信息用普通 `html2d`。
- 需要跨 world 或 agent-to-agent 交换：用 `handoff`，把 lens/bucket/message envelope 说清楚。

## Render mode 的完成条件

Render mode 只有在实际调用 `render_view` 或 `render_status` 后才算完成。

- 你可以先描述视觉方案，但这只是 plan。
- 要让 3030 浏览器变化，必须调用 `render_view`。
- 要让 macOS/status surface 变化，必须调用 `render_status`。
- 如果最终回复前没有工具调用，不要说“已经更新”；只能说“我设计了，但还未渲染”。

## 轻量 ECS schema 示例

如果世界里还没有类似组件，可以定义轻量组件。字段保持少，不要做大型框架。

```json
{
  "name": "AgentMode",
  "schema": {
    "mode": "string",
    "intent": "string",
    "status": "string",
    "reason": "string",
    "selected_at": "integer",
    "owner": "string"
  }
}
```

然后写入当前选择：

```json
{
  "components": [
    {
      "name": "AgentMode",
      "value": {
        "mode": "projection",
        "intent": "record clipboard image hash and OCR for later query",
        "status": "running",
        "reason": "input is an image source that should be indexed, not kept only in chat",
        "selected_at": 1786780000,
        "owner": "coordinator"
      }
    }
  ],
  "description": "current agent mode selection"
}
```

如果已有 `WorkItem`、`Plan`、`TransactionRecord`、`ActionRecord` 或项目自己的动态组件，优先复用。

## 从 mode 到 Rhai system / TickAction

当一个 mode 变成可重复的规则，不要每次只靠口头推理。把它沉淀成 system：

```text
run_system("guide_rhai_syntax")
define_system(id="project_clipboard_images", source="...")
run_system("project_clipboard_images")
```

当一个 mode 需要长期观察世界阈值，改用 TickAction：

```text
run_system("guide_tick_action")
spawn_entity(TickAction { script: "... if condition { push_spark(...) } ...", cooldown_secs: 300 })
```

边界：

- `define_system` / `run_system` 是明确执行。
- `TickAction` 是 runtime 每 tick 的背景触发。
- `push_spark` 是把后续语义任务交回 LLM。
- ECS 里的 mode record 是可观察状态，不等于自动执行；执行仍然要靠 tool、system、TickAction 或下一 Spark。

## 给用户看见

如果 mode 影响用户等待或选择，发布 UI：

- `render_status`：当前模式、进度、下一步选择。
- `render_view` 普通 `html2d`：计划表、检查结果、关系索引。
- `render_view` `mind-view`：动态视觉或概念场，不放长文档。

示例：当切到 `approval`，不要只问一句话；发布 choice items，让用户点击。

## 重要原则

- Mode 是帮助你选择执行颗粒，不是多写一段解释。
- 不要为每个 Spark 都创建 mode entity；只有 mode 会影响后续状态、UI、脚本或长期行为时才落 ECS。
- 不要让 TickAction 做复杂语义判断；复杂语义通过 spawn Spark 交给 agent。
- 不要把可读事实只留在对话里；需要复用的事实投影到 ECS。
- 不确定 schema 时先 `get_world_overview` / `query_entities`，不要猜字段和类型。
