# 本地 Lens 应用与对话结果

本地 Lens 是持久化在 ECS 的上下文处理定义。现有跨世界 Lens 交换协议保持独立。

任务发现、使用反馈与候选演进见 [Lens 学习流程](lens-learning.md)。

## Agent 工具

- `define_lens_app`：创建定义，或以 `expected_version` 更新定义。更新必须采用新版本。
- `run_lens_app`：读取当前 ECS 并运行组合，返回观察结果及来源引用。
- `inspect_lens_apps`：列出定义；指定 `id` 可读取定义和最近一次成功运行的回执。
- `define_lens_binding`：创建或更新推理前上下文绑定，更新需 `expected_revision`。
- `inspect_lens_bindings`：检查各绑定及独立运行实例，`details=true` 包含历史成功回执。

LensApplication 保存定义和最近成功回执，随 checkpoint 恢复。失败记录走现有工具审计。
定义包含 `id`、`version`、`purpose`、`reads`、`stage`。reads 是组件名称；完整组合的读取范围是所有依赖的 reads 并集。

## 可运行例子

先调用 define_lens_app：

```json
{
  "definition": {
    "id": "weather.schedule",
    "version": "1",
    "purpose": "观察六小时刷新任务的最近执行时间",
    "reads": ["TickAction"],
    "stage": {
      "kind": "sequence",
      "stages": [
        {"kind": "filter", "pointer": "/TickAction/cooldown_secs", "equals": 21600},
        {"kind": "project", "fields": {"script": "/TickAction/script", "last_run": "/TickAction/last_run"}}
      ]
    }
  }
}
```

然后定义叠加应用：

```json
{
  "definition": {
    "id": "weather.workspace",
    "version": "1",
    "purpose": "同时提供完整任务和简要执行时间",
    "reads": [],
    "stage": {
      "kind": "sequence",
      "stages": [
        {"kind": "lens", "id": "weather.schedule", "version": "1"},
        {"kind": "parallel", "branches": {
          "detail": {"kind": "identity"},
          "brief": {"kind": "project", "fields": {"last_run": "/last_run"}}
        }}
      ]
    }
  }
}
```

调用 `run_lens_app({"id":"weather.workspace"})`。
parallel 对同一份输入分别计算，按分支名稳定汇合，保留 branches 标记；当前不创建并发线程。
sequence 按顺序传递结果，JSON Pointer 总是相对当前阶段输入。缺失的投影字段为 null。
这只是按周期筛选的示例，不足以证明任务一定查询天气；实际应用应使用业务标识进一步限定。

## 执行约定

每个事实携带稳定 source_id 和内容版本，投影保留来源；每次结果有 run_id 和 observed_at。
observed_at 是本次读取时间，不是网页发布时间，不能据此把旧数据标为最新。
来源版本使用非密码学内容摘要作为变化标记。details=true 返回输入版本及 catalog_versions。

第一版所有阶段只读业务数据，运行时仅维护身份和回执。业务修改继续由操作工具执行。
stage 是 Rust tagged enum，支持 identity/filter/project/take/sequence/parallel/lens。
不支持把任意脚本或自然语言当作阶段代码执行。

有界约束：128 个定义、128 个展开阶段、16 层 Lens 引用、256 条输入或输出；超限显式报错。
单个源事实最多 64 KiB，运行回执最多 256 KiB。大数据先组织为 shard 或更窄的业务组件。
输入和每个中间阶段最多 1 MiB（序列化大小），投影在复制字段前检查预算，防止组合层层放大内存。
引用必须命中明确版本；环路、版本不匹配和未知组件都会返回错误。
更新被引用定义若会破坏已有组合会被拒绝，可用新 id 并存发布，再升级上游。

## 自动上下文绑定

三个概念已分开：LensDefinition 表达组合规则；LensBinding 表达触发点、目标 agent、固定 Lens 版本和启停配置；LensInstance 表达一个绑定的运行状态。
LensApplication 继续保存定义及手动运行回执，兼容已有 checkpoint；自动运行不覆盖这份手动回执。
LensBinding 与 LensInstance 放在同一实体上，以 binding id 关联，不用易变的实体索引作为业务身份。

目前只支持 `agent=coordinator`、`trigger=before_reasoning`，输出固定为初始上下文。
工具注册只暴露能力，不会执行 Lens。只有显式启用的绑定才会在 Spark 被承接、首次模型调用之前执行；轻量对话和工具任务均适用。
一次 cycle 内不随 wave 重跑。同一 cycle 再次准备上下文会复用该绑定的本轮结果；下一 cycle 重新读取事实，不缓存跨 cycle 的结果。
绑定在工具 wave 中修改后，下一个 cycle 生效；暂无 agent 子系统、view 路由、ECS 变化订阅或定时触发。

例如在前面的 weather.schedule 基础上定义简短上下文，避免把整段 Rhai 脚本注入对话：

```json
{
  "definition": {
    "id": "weather.context",
    "version": "1",
    "purpose": "为对话提供天气刷新任务的上次运行时间",
    "reads": [],
    "stage": {"kind": "sequence", "stages": [
      {"kind": "lens", "id": "weather.schedule", "version": "1"},
      {"kind": "project", "fields": {"last_run": "/last_run"}}
    ]}
  }
}
```

随后调用 `define_lens_binding`：

```json
{
  "binding": {
    "id": "chat.weather",
    "lens_id": "weather.context",
    "lens_version": "1",
    "agent": "coordinator",
    "trigger": "before_reasoning",
    "enabled": true,
    "priority": 10
  }
}
```

首次创建返回 revision=1。暂停时提交完整 binding、设置 enabled=false，并在外层携带 expected_revision=1。
每次配置更新增加 revision，清空该实例的当前回执与错误，保留累计 attempts。enabled 缺省为 false。
没有自动创建、激活或迁移绑定的启动逻辑；旧世界恢复后仍保持原有行为，直到 agent 显式创建绑定。

实例状态为 ready/paused/succeeded/failed/budget_skipped，可查看 attempts、last_cycle_id、observed_at、last_error。
这里的 last_cycle_id 是最近尝试的 cycle，不是 ECS 事件消费游标；observed_at 是最近尝试读取的时间。
失败保留历史成功回执供检查，但只向本次模型输入诊断，绝不注入旧成功数据。下个新 cycle 可再次尝试，不追加 Spark，不后台自旋。
固定版本失配会失败，需要显式升级绑定；即使定义已损坏，也允许暂停绑定。

每个世界最多 32 个绑定；每轮按 priority 降序、id 升序运行前 8 个，其他实例标记 budget_skipped。
每个上下文观察最多 2 KiB。输出 data 为投影后的字段数组，不再包装每条 Fact 的 value；完整来源仍保留在详细 receipt。
超限时原始 receipt 自动切成不超过 16 KiB 的 UTF-8 安全 ShardData，context 返回 receipt_ref。
通过 get_shard_content 的 paradigm、shard_index、offset、limit 分页读取；按 shard_index 拼接可还原完整 JSON。
每次写入使用独立引用，不覆盖已有分片；目前尚未提供自动回收或内容去重。
这些限额只约束 Lens 注入部分，不代表整个 agent 上下文上限。长期低优先级绑定可能持续被跳过，应暂停无用绑定或合并投影。
运行时只维护实例、来源身份与回执等执行元数据；业务事实和 Lens 配置仍由 agent 经工具写入，不自动生成业务结论。

## Rust 模块边界

### 时效配置

Binding 可配置 source_updated_at_pointer（例如 /SessionContext/updated_at_epoch，值必须是 Unix 秒）与 stale_threshold_ms。
freshness.age_ms 使用本轮读取时间减去最早的来源更新时间，乘以 1000；stale 表示超过配置阈值。
无配置、空来源、缺失时间或未来时间均报告 unknown，而不是 fresh。多来源的时效按保守聚合计算。
Lens 只读 ECS：重新运行 Lens 不会自动 fetch 新天气，过期时应刷新来源再观察。
这些字段 serde 缺省为空，旧 Binding 无需迁移，不自动修改线上业务数据。

### 覆盖边界

context_artifact 是 Lens 与模型出口投影共用的原文存储层。模型出口新增聚合预算回退，优先投影较大的工具结果内容，保留 ToolUse/ToolResult 的 ID 配对。
它不代表任意输入都能缩到任意预算：不可压缩的控制信息仍可能触发硬上限，Lens 的源事实与执行内存限制也仍生效。
事件触发、仪表盘、分片回收，以及系统提示和工具 schema 的投影策略整理仍为后续工作。

### 显式来源缓存与状态摘要

Binding 的 refresh_interval_ms 默认 0（不缓存）；设为 300000 可启用最长 5 分钟的 stage 复用，上限一天。
更新仍需提交完整 binding 和 expected_revision，不会替用户自动启用线上绑定。
每轮仍读取并序列化来源以计算 source_version；输入实体集合、来源版本、目录定义内容或时效配置变化都使缓存失效。
这不是 ECS 事件订阅，也不省掉来源扫描，只跳过纯 Rust stage 执行；暂无外部 API/DB Lens stage。
TTL 从 executed_at（上次实际执行）计算，而非 observed_at（本轮读取）；命中不会延长 TTL。
缓存命中保留 run_id，重新计算 freshness，可能同时 cache_hit=true、stale=true；这表示投影可复用但来源已旧，需要刷新来源。
失败后保留历史 receipt 供 inspect，但下一次必须重新执行，不能从失败状态复用成功数据。
大回执缓存命中复用既有分片引用，分片中的 observed_at 是原执行时间，以本轮 context 的 freshness 为准。

context 中新增 binding_status：列出本轮所有启用且属于该 agent 的绑定，包括被预算跳过的 id、status、cache_hit 与有界 last_error。
暂停绑定仍通过 inspect_lens_bindings 检查。LensInstance 记录 cache_hits/last_cache_hit；attempts 仍是尝试轮次，不能当作实际 stage 执行次数。
旧 checkpoint 缺少新增字段时使用 serde 缺省值，不改变原启停与调度行为。

- big-head-world/application：与 ECS/LLM 无关的纯组合、依赖校验和中间数据预算。
- big-head-ecs/lens：可反射、可持久化的 Application、Binding、Instance 及类型化触发/状态枚举。
- big-head-loop/lens_runtime：共享目录读取与事实采集执行，手动工具和自动绑定复用，不模拟工具调用。
- big-head-loop/lens_bindings：绑定管理、推理前调度、实例状态和有界上下文生成。
- runtime 仅在初始上下文准备处调用调度模块，agent 工具接入现有分发及权限过滤。

## 对话与操作

用户交互工具任务完成后，额外运行无工具的对话阶段，根据操作回执解释结果。
简单对话继续直接回答；后台回执保留在 Spark、运行日志和 TUI Runtime，不进入聊天列表。
后台操作不会作为用户历史混入下一轮上下文。
该阶段每个交互工具任务增加一次模型调用，目前是结果解释层，尚未实现独立策略规划与委派协议。

尚未实现：自动依赖变化订阅、Lens 内 LLM/写入阶段、图形组合编辑器、通用有效期推导。
本次定义与执行工具可以作为这些能力的基础，不能把只读观察结果当作动作完成凭证。
