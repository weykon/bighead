# Token Breakdown 可视化功能

## 📊 新增功能

每次API call之前，runtime会自动emit详细嘅token breakdown，显示context window嘅组成。

## 🎯 解决嘅问题

之前你提出嘅问题：
- ❓ "几多KB嘅内容发去LLM？"
- ❓ "每次API call嘅组成系点样？"
- ❓ "Spark原文、工具结果、历史对话各占几多token？"

依家**全部可见**！

## 🔧 实现细节

### 1. 新增 `AgentActivity::TokenBreakdown` event

**位置**：`crates/big-head-state/src/activity.rs`

```rust
TokenBreakdown {
    wave: usize,                      // 第几轮
    system_prompt_tokens: usize,      // System prompt占用
    initial_context_tokens: usize,    // messages[0]: Spark + 世界快照
    conversation_tokens: usize,       // messages[1..]: assistant + tool_result历史
    tool_definitions_tokens: usize,   // 12个ECS工具 + Rhai动态工具
    total_estimated: usize,           // 总计
    message_count: usize,             // messages数量
    context_usage_pct: f32,           // 使用百分比
}
```

### 2. Agent每轮自动emit breakdown

**位置**：`crates/big-head-ai/src/agent.rs:184-221`

每次调用LLM之前，agent会：
1. 重置token budget
2. 计算各部分token数：
   - System prompt
   - Initial context (messages[0])
   - Conversation history (messages[1..])
   - Tool definitions
3. Emit `TokenBreakdown` event
4. 检查budget status

### 3. Terminal显示

**位置**：`crates/big-head-state/src/render.rs:52-69`

输出格式：
```
  📊 Wave 2 Token Breakdown (3 messages, 45.2% used):
     System Prompt:       2000 tokens
     Initial Context:     5000 tokens  (Spark + World)
     Conversation:       40000 tokens  (History)
     Tool Definitions:    2400 tokens
     Total:              49400 tokens
```

## 📈 使用场景

### 场景1：发现token爆炸

```
Wave 1 Token Breakdown (2 messages, 8.5% used):
  System Prompt:       2000 tokens
  Initial Context:     5000 tokens  (Spark + World)
  Conversation:            0 tokens  (History)
  Tool Definitions:    2400 tokens
  Total:               9400 tokens

Wave 2 Token Breakdown (4 messages, 28.7% used):
  System Prompt:       2000 tokens
  Initial Context:     5000 tokens  (Spark + World)
  Conversation:       50000 tokens  (History)  ← ⚠️ 爆炸！
  Tool Definitions:    2400 tokens
  Total:              59400 tokens
```

**诊断**：Wave 1嘅tool result（query_entities返回80个entities）占咗50KB，需要自动shard。

### 场景2：正常增长

```
Wave 1: 9400 tokens (4.7%)
Wave 2: 12500 tokens (6.3%)
Wave 3: 15800 tokens (7.9%)
Wave 4: 18200 tokens (9.1%)
```

**诊断**：每轮增长~3KB，健康嘅conversation flow。

### 场景3：World snapshot过大

```
Wave 1 Token Breakdown (2 messages, 15.2% used):
  System Prompt:       2000 tokens
  Initial Context:    28000 tokens  (Spark + World)  ← ⚠️ World太大
  Conversation:            0 tokens
  Tool Definitions:    2400 tokens
  Total:              32400 tokens
```

**诊断**：World snapshot已经占15%，需要用query filter缩窄，唔好`get_world_overview`全dump。

## 🚀 下一步

依家有咗可视化，我哋可以：

1. **加自动shard机制** — 当conversation_tokens单次增长 > 10KB，自动转shard
2. **优化initial context** — 如果world snapshot > 20KB，提示用户用query filter
3. **历史压缩** — 当conversation > 100KB，自动触发shard_content压缩历史

## 🧪 如何测试

```bash
cargo run --release

# 输入会产生大量entities嘅spark：
> 统计所有订单

# 观察terminal输出：
  📊 Wave 1 Token Breakdown (2 messages, 4.5% used):
     System Prompt:       2000 tokens
     Initial Context:     5000 tokens  (Spark + World)
     Conversation:            0 tokens  (History)
     Tool Definitions:    2400 tokens
     Total:               9400 tokens

  🧠 Wave 1 ── reasoning
  📡 API → Anthropic (claude-opus-4-7)
  🔧 Wave 1: query_entities ({"query_string": "BusinessOrder"})
    ✓ query_entities ── Entity 1: {...}\nEntity 2: {...}...

  📊 Wave 2 Token Breakdown (4 messages, 28.7% used):
     System Prompt:       2000 tokens
     Initial Context:     5000 tokens  (Spark + World)
     Conversation:       50000 tokens  (History)  ← ⚠️ 注意呢度！
     Tool Definitions:    2400 tokens
     Total:              59400 tokens
```

## 📝 代码改动总结

| 文件 | 改动 | 行数 |
|------|------|------|
| `crates/big-head-state/src/activity.rs` | 加`TokenBreakdown` variant | +9 |
| `crates/big-head-ai/src/agent.rs` | 计算breakdown + emit event | +37 |
| `crates/big-head-state/src/render.rs` | 渲染breakdown到terminal | +18 |
| **总计** | | **64行** |

## ✅ 完成状态

- ✅ `TokenBreakdown` event定义
- ✅ Agent自动计算breakdown
- ✅ Terminal渲染
- ✅ Workspace编译通过
- ⏳ 待测试：运行时验证
- ⏳ 待实现：自动shard大tool result（下一步）

---

**Created**: 2026-06-13  
**Author**: Claude Opus 4.7 + weykon  
**Status**: ✅ Ready for testing
