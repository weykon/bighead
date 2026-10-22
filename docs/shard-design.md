# big-head-shard: 上下文分片设计方案 v3

## 问题场景

一个 spark 进入，agent 多轮 wave 执行工具，每轮 tool result 直接追加到 message history：

```
Spark: "分析项目结构"
  Wave 1: get_world_overview → 小结果
  Wave 2: query_entities 返回 200 个实体 → 大 JSON
  Wave 3: execute_cli "cat large.csv" → 5000 行输出
  Wave 4: execute_cli "npm install" → 3000 行日志
  Wave N: TokenBudget → Exceeded → 停了，后面信息全丢 ✗
```

当前 TokenBudget 的三种状态（`external/ai_api_provider/src/tokens.rs`）：

| 状态 | 条件 | 当前处理 |
|------|------|----------|
| `Ok` | used < 80% limit | 继续 |
| `Warning` | used >= 80% limit | 打 log + 追加提示，继续 |
| `Exceeded` | used > limit | 追加系统提示到消息历史，Agent 决定是否分片 |

### TokenBudget 检查位置

在 `crates/big-head-ai/src/agent.rs`，**每轮 wave 前**重算一次。

### Token 估算器

使用 `estimate_tokens(text)`（`external/ai_api_provider/src/tokens.rs`）：
- 启发式算法：ASCII 字符 ÷ 3，CJK 字符 × 2/3
- 误差 ~10%，零外部依赖，足够用于预算检查

`count_tokens_precise` 和 `tiktoken-rs` 已删除。

---

## 核心设计：Agent 调度 + 基础设施执行的 Map-Store 分片

### 设计原则

**Agent 管战略（怎么切、怎么分析），基础设施管执行（切分、LLM 调用、存储）。**

分片不是纯文本切分，而是 LLM 参与处理每个分片，提取事实后独立存储。**不做 Reduce 合并** — 每个分片的事实作为独立的 ShardData 组件写入 ECS，保持事实保真。

### 完整流程

```
Tool 返回大结果 (例如等效 400K tokens)
  │
  ├─ Agent 检测到 TokenBudget Warning/Exceeded
  │   决定发起分片 — 调用 shard_content 工具
  │
  ├─ Agent 指定:
  │   - content_ref: 大内容的引用（"__last_tool_result"）
  │   - strategy: 切分策略 (overlap / custom_ranges / head_tail / custom)
  │   - map_prompt 模板: LLM 处理每个分片时的指令
  │   - paradigm: 范式标识
  │   - 策略相关参数（overlap_size, ranges, head_lines, tail_lines, custom_split_description）
  │
  ├─ 基础设施执行:
  │   ├─ 1. 按策略切分文本
  │   ├─ 2. 对每个分片独立请求 LLM:
  │   │     map_prompt 模板展开 {shard_index}, {total}, {overlap_note}
  │   │   → LLM 提取该分片的确凿事实
  │   ├─ 3. 每个分片结果写入 ECS:
  │   │     world.spawn(ShardData {
  │   │         paradigm: "tool:execute_cli:abc123",
  │   │         shard_index: 0, total: 3,
  │   │         content: "发现项目使用 Actix Web 框架..."
  │   │     })
  │   └─ 4. 返回简短通知给 Agent:
  │         "Content sharded into 3 parts. Paradigm: 'tool:execute_cli:abc123'
  │           Query with: get_shards paradigm=tool:execute_cli:abc123
  │           or query_entities component=ShardData"
  │
  └─ Agent 继续推理，需要时按 paradigm + shard_index 查询具体分片
```

### 为什么不做 Reduce

- **事实保真**：每个分片独立存储，不会在合并中丢失细节
- **按需检索**：Agent 按需查询某几个分片，不用全部加载
- **可追溯**：每个分片的来源清晰，知道哪个工具调用、第几部分

---

## shard_content 工具定义

Agent 通过 `shard_content` 工具主动发起分片。

### 工具 Schema

```json
{
  "name": "shard_content",
  "description": "将大段内容分片，每个分片由 LLM 独立分析提取事实，结果存入 ECS ShardData 组件。",
  "input_schema": {
    "type": "object",
    "properties": {
      "content_ref": {
        "type": "string",
        "description": "大内容的引用标识，当前支持 '__last_tool_result'"
      },
      "strategy": {
        "type": "string",
        "enum": ["overlap", "custom_ranges", "head_tail", "custom"],
        "description": "切分策略：overlap=重叠窗口，custom_ranges=指定行号范围，head_tail=头尾完整+中间切片，custom=用自然语言描述切分方式"
      },
      "paradigm": {
        "type": "string",
        "description": "范式标识，用于后续查询归属同一来源的分片"
      },
      "map_prompt": {
        "type": "string",
        "description": "LLM 处理每个分片的指令模板。可用变量：{shard_index}, {total}, {overlap_note}"
      },
      "overlap_size": {
        "type": "integer",
        "description": "overlap 策略的窗口重叠行数，默认 50"
      },
      "ranges": {
        "type": "array",
        "description": "custom_ranges 策略的行号范围列表 [[start, end], ...]"
      },
      "head_lines": {
        "type": "integer",
        "description": "head_tail 策略的头部保留行数，默认 50"
      },
      "tail_lines": {
        "type": "integer",
        "description": "head_tail 策略的尾部保留行数，默认 50"
      },
      "custom_split_description": {
        "type": "string",
        "description": "custom 策略的自然语言切分说明（strategy=custom 时必填）"
      }
    },
    "required": ["content_ref", "strategy", "paradigm", "map_prompt"]
  }
}
```

### 鼓励自定义策略

Agent 应根据内容性质选择最适合的策略。**优先使用 `custom` 策略**，用自己的话描述如何切分内容，而不是依赖固定算法。例如：

- "将 JSON 数组按每 50 个元素切分"
- "按日志时间戳的自然段切分"
- "将 CSV 按列为单位分组，每组包含 header + 3 列数据"

基础设施无法理解内容的语义结构，只有 Agent 可以。

## get_shards 工具定义

专用查询工具，按 paradigm 检索 ShardData。

```json
{
  "name": "get_shards",
  "description": "查询某个范式下的所有分片，按 shard_index 排序返回",
  "input_schema": {
    "type": "object",
    "properties": {
      "paradigm": {
        "type": "string",
        "description": "范式标识"
      }
    },
    "required": ["paradigm"]
  }
}
```

ShardData 也可以通过现有 `query_entities component=ShardData` 通用工具查询。

---

## 切分策略

### 策略 1：overlap（重叠窗口）

解决硬切分导致的上下文断裂问题。

```
原始内容:
  行 0-99:   ...调用 build_pipeline()...
  行 100-199: fn build_pipeline() { ... }
  行 200-299: ...更多内容...

overlap_size=50 切分:
  分片 0: 行 0-149    (末尾 50 行与分片 1 共享)
  分片 1: 行 100-249  (开头 50 行 = 分片 0 末尾, 末尾 50 行 = 分片 2 开头)
  分片 2: 行 200-299  (开头 50 行与分片 1 共享)
```

模板变量 `{overlap_note}` 自动展开为重叠提示。

### 策略 2：custom_ranges

Agent 自行指定精确的行号范围，用于结构化内容。

```
ranges: [[0, 99], [100, 199], [200, 299]]
```

### 策略 3：head_tail

头尾完整 + 中间分片。适用于日志类内容——开头（启动信息）和结尾（结果/错误）最重要，中间可以切片。

```
原始: 5000 行
分片 0: 行 0-99    (头部完整)
分片 1: 行 100-599 (中间切片 1)
分片 2: 行 600-1099(中间切片 2)
...
分片 N: 行 4900-4999(尾部完整)
```

### 策略 4：custom

Agent 用自然语言描述切分方式，基础设施无法理解语义结构，只有 Agent 可以根据内容性质自定义逻辑。

---

## ShardData 组件

### 选型：Rust 原生组件

ShardData 和 ToolHistory 一样走 Rust 原生组件路线，在 `crates/big-head-shard/src/component.rs` 中定义。

```rust
#[derive(Component, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ShardData {
    pub paradigm: String,
    pub shard_index: u32,
    pub total_shards: u32,
    pub content: String,
    pub source_label: String,
    pub created_at: u64,
}
```

### 为什么不用动态组件

- 字段结构固定 — paradigm / shard_index / total_shards / content / source_label / created_at 对所有场景通用
- 查询模式固定 — 按 paradigm 聚合、按 shard_index 排序
- 如果未来需要扩展字段，加 `metadata: serde_json::Value` 即可

### 注册

```rust
// AgentRuntime::register_default_components()
rt.register_component_action::<ShardData>("ShardData");
```

---

## 两个查询入口

| 方式 | 用途 |
|------|------|
| `get_shards(paradigm)` | 专用工具，按 paradigm 聚合 + 按 shard_index 排序，带预览截断 |
| `query_entities component=ShardData` | 通用工具，支持 DSL 过滤和分页 |

---

## 适用范围

同一套分片机制服务于不同场景，区别仅在于 Agent 写的 `map_prompt`：

| 场景 | paradigm 示例 | map_prompt 侧重点 |
|------|------|------|
| Tool 结果过大 | `tool:execute_cli:abc123` | 提取与工具意图相关的事实 |
| Spark 任务过大 | `spark:analysis_20260530` | 提取关键目标和约束条件 |
| 消息历史压缩 | `history:wave_3_results` | 提取关键决策、发现和当前状态 |
| Rhai 脚本输出 | `rhai:health_check:log` | 提取脚本执行中的异常和结果 |
| 大实体查询 | `query:entities_batch_1` | 提取实体关键属性和关系 |

---

## 分层应对策略

| 阶段 | 条件 | 行为 |
|------|------|------|
| 正常 | used < 80% | 直接追加 tool_result |
| 预警 | used >= 80% (Warning) | 追加提示到消息历史，Agent 可考虑分片 |
| 超限 | used > limit (Exceeded) | 追加系统提示，Agent 决定分片或终止 |

不再硬终止循环。Agent 看到提示后自行决定是否调用 `shard_content`。

---

## 实际代码位置

| 位置 | 内容 |
|------|------|
| `crates/big-head-shard/src/component.rs` | ShardData 组件定义 |
| `crates/big-head-shard/src/sharder.rs` | ShardConfig, ShardStrategy, Sharder (split/calculate_shards/expand_prompt) |
| `crates/big-head-ai/src/tool_definitions.rs` | shard_content + get_shards 工具定义 |
| `crates/big-head-ai/src/actions.rs` | ShardContent action 变体 |
| `crates/big-head-ai/src/agent.rs` | Exceeded/Warning 时追加通知，替代直接终止 |
| `crates/big-head-loop/src/tools/shard.rs` | dispatch_shard_content + dispatch_get_shards |
| `crates/big-head-loop/src/tools/mod.rs` | shard 模块路由 + RuntimeToolExecutor 集成 |
| `crates/big-head-loop/src/runtime.rs` | ShardData 注册 + Sharder 初始化 |
| `crates/big-head-loop/src/registry.rs` | ShardContent action match 分支 |
| `prompts/coordinator.md` | shard_content + get_shards 工具说明 |
