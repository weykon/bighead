# Step 2: TickAction — Agent 自主触发嘅机制

> **Status**: 设计阶段，尚未实装
> **Branch**: `feature/tick-loop-step1` (Step 1 已 commit) → 即将开 `feature/tick-action-step2`
> **Last updated**: 2026-06-01

---

## 0. 背景同问题

### 现有架构做唔到嘅嘢

big-head 而家系**事件驱动**（event-driven）——只有当外部有 input（user 打字 / push_spark）先会跑 agent。**Agent 永远唔会"自己醒嚟"**。

呢个系**关键缺口**：

> **Agent 缺乏"跨时空"嘅自主触发能力**——佢只能响应"而家发生嘅事"，**冇办法主动安排"将来发生嘅事"**。

### Step 1 已实装

- Tick loop（50ms）
- Reader thread + mpsc channel
- 行为不变（user 体验完全一致）

---

## 1. 设计原则

### 1.1 唔发明新概念

之前讨论过 Sensor / Event / Subsystem 等新概念。**已确认：drop 晒**。理由：

- 现有 Rhai 引擎已经够灵活
- 现有 ECS query DSL 已经够表达
- Sensor / Event 嘅功能可以由**Rhian script + 现有工具**完全实现
- 加新 component 会增加架构负担

### 1.2 TickAction 嘅独特定位

TickAction 唔系"新嘅触发机制"——佢系**唯一由 runtime 自主触发嘅机制**。

| 机制 | 谁触发 | 时机 |
|------|--------|------|
| **TickAction** ⭐ | runtime（每 tick） | **自主** |
| `run_system` | agent / LLM | 主动调用 |
| `define_tool` | agent | 注册后俾 LLM 用 |
| `push_spark` | user | 用户输入 |

**TickAction 系唯一冇"外部 trigger"嘅机制**。

---

## 2. 触发条件嘅 8 个范畴（之前讨论过）

TickAction.script 嘅 Rhai 表达式可以表达**任何**观察世界嘅条件。具体范畴：

| 范畴 | 例子 | 实质 |
|------|------|------|
| **时间类 (Temporal)** | `now() % 3600 < 60` | 时间驱动 |
| **事件类 (Reactive)** | `has_unconsumed_event('UserInput')` | 事件驱动 |
| **状态类 (Stateful)** | `entity_count('SparkRecord') > 50` | 状态驱动 |
| **资源类 (Resource)** | `token_budget() < 0.2` | 资源监控 |
| **关系类 (Relational)** | `agent_has_no_pending_spark()` | 关系判定 |
| **时间+状态 (Hybrid)** | `now() - last_action > 300 && has_event('X')` | 复合 |
| **反身类 (Reflexive)** | `my_last_thought > 60` | 内观自己 |
| **涌现类 (Emergent)** | `cluster_agents(2).any_inactive()` | 集群判定 |

**关键**：全部都系 query world state——**TickAction 唔做语义判断，只做"现在世界系咩样"嘅判定**。

---

## 3. TickAction Component 设计

### 3.1 字段（3 个）

```rust
//! crates/big-head-ecs/src/components/tick_action.rs

use bevy_ecs::component::Component;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

/// 标记 entity 喺每个 tick 自动跑一段 Rhai 脚本。
///
/// TickAction 系 agent 定义"跨时空触发"嘅唯一方式——
/// 唔需要外部 prompt、唔需要工具调用，runtime 自己每 tick 评估。
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize)]
pub struct TickAction {
    /// 每次 tick 跑嘅 Rhai 脚本。脚本可以 query world、spawn spark、
    /// modify_component、emit log。
    pub script: String,

    /// 上次跑嘅时间（epoch secs）。0 = 从未跑过。
    pub last_run: u64,

    /// 冷却时间（秒）。`now - last_run >= cooldown_secs` 才会跑。
    pub cooldown_secs: u64,
}

impl Default for TickAction {
    fn default() -> Self {
        Self {
            script: String::new(),
            last_run: 0,
            cooldown_secs: 60,
        }
    }
}
```

### 3.2 字段设计决策

| 决策 | 原因 |
|------|------|
| **冇 priority 字段** | v1 所有 TickAction 等价，先入先跑 |
| **冇 enabled 字段** | disable 就 despawn entity |
| **冇 prepare script 字段** | mutation 应该走现有工具（modify_component 等） |
| **冇 hit_count 字段** | 暂时唔需要统计；如需要 v2 加 |
| **冇 name / description 字段** | LLM 可以 query `get_entity` 睇 raw 字段 |
| **cooldown_secs 强制 ≥1** | v1 不强制（YAGNI），未来如果死循环再加 |

---

## 4. Tick Loop 嘅 TickAction 评估

### 4.1 现有 main loop 结构（Step 1）

```rust
loop {
    // 1. user input handling
    match rx.try_recv() { ... }

    // 2. 心跳
    tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
}
```

### 4.2 Step 2 加入 TickAction 评估

```rust
loop {
    // 1. user input handling
    match rx.try_recv() { ... }

    // 2. 新增：TickAction 评估
    evaluate_tick_actions(&mut rt);

    // 3. 心跳
    tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
}
```

### 4.3 evaluate_tick_actions 嘅伪代码

```rust
fn evaluate_tick_actions(rt: &mut AgentRuntime) {
    let now = now_epoch_secs();

    // 1. 收集需要跑嘅 actions（避免 borrow 冲突）
    let actions: Vec<(Entity, String)> = {
        let world = rt.world();
        world.query::<(Entity, &TickAction)>()
            .iter(world)
            .filter_map(|(e, a)| {
                if now.saturating_sub(a.last_run) >= a.cooldown_secs {
                    Some((e, a.script.clone()))
                } else {
                    None
                }
            })
            .collect()
    };

    // 2. 逐个跑脚本
    for (entity, script) in actions {
        if let Err(e) = rt.eval_rhai(&script) {
            eprintln!("TickAction {} script error: {}", entity, e);
        }
        // 3. 更新 last_run
        if let Ok(mut em) = rt.world_mut().get_entity_mut(entity) {
            if let Some(mut action) = em.get_mut::<TickAction>() {
                action.last_run = now;
            }
        }
    }
}
```

---

## 5. TickAction 嘅生命周期

### 5.1 创建（startup / agent）

**路径 A：startup 时 hardcode**

```rust
// main.rs 启动时
rt.world.spawn(TickAction {
    script: "if has_unconsumed_user_input() { log('wake') }".into(),
    last_run: 0,
    cooldown_secs: 30,
});
```

**路径 B：agent 喺 reasoning 中创造**

```
User: "每 30 分钟自动提醒我喝水"

Agent reasoning:
  "我需要创建一个 TickAction 让 runtime 每 30 分钟执行提醒"
  → spawn_entity() 创建一个 entity
  → modify_component() 挂上 TickAction
  → 新 TickAction 出现，runtime 下个 tick 开始评估
```

### 5.2 修改

Agent 可以用 `modify_component` 改 TickAction 嘅任何字段：

- `script` —— 改触发条件
- `cooldown_secs` —— 改触发频率
- `last_run` —— 强制 reset（让下个 tick 立即跑）

### 5.3 删除

Agent 可以用 `despawn_entity` 删除 TickAction entity——TickAction 立即停止。

---

## 6. Rhai 脚本可以做到嘅嘢

TickAction 嘅 script 喺 Rhai 引擎中执行，**所有现有 Rhai bridge function 都可用**。

### 6.1 现有 Rhai 桥接（待确认）

需要确认 `crates/big-head-script/src/scripting.rs` 暴露咗咩 function：

- `all_entities()`, `entities_with()`, `has_component()`, `get_component()`
- `set_component()`, `remove_component()`
- `query()`, `spawn_entity()`, `despawn()`
- `log()`, `timestamp()`
- `define_component()`, `define_system()`, `define_tool()`

**确认后会喺 Step 2 入面补完**。

### 6.2 脚本可以做到

- ✅ Query world
- ✅ Spawn / despawn entity
- ✅ Modify / remove component
- ✅ Emit log
- ✅ Spawn spark（如果 script 入面 call `push_spark`）
- ❌ **唔可以**：直接调 LLM（必须 spawn spark 先可以触发 agent 推理）
- ❌ **唔可以**：跨 tick 保留局部变量（每次都系新 scope）

---

## 7. 3 个 Step 2 嘅验证用 TickAction

```rust
// main.rs 启动时注册

// 1. 时间驱动
rt.world.spawn(TickAction {
    script: r#"log("tick fired at " + timestamp())"#.into(),
    last_run: 0,
    cooldown_secs: 10,  // 每 10 秒
});

// 2. 状态驱动
rt.world.spawn(TickAction {
    script: r#"
        let count = entities_with("SparkRecord").len();
        if count > 50 {
            log("⚠️ too many SparkRecord: " + count);
        }
    "#.into(),
    last_run: 0,
    cooldown_secs: 60,
});

// 3. 资源驱动
rt.world.spawn(TickAction {
    script: r#"
        let budget = get_component(agent_entity(), "TokenBudget");
        if budget != () && budget.remaining < 1000 {
            log("Token budget low: " + budget.remaining);
        }
    "#.into(),
    last_run: 0,
    cooldown_secs: 30,
});
```

---

## 8. 验证清单（Step 2 完成标准）

- [ ] `TickAction` component 创建并注册到 ECS
- [ ] `evaluate_tick_actions` 函数实装喺 runtime 或 main.rs
- [ ] Tick loop 集成咗 TickAction 评估
- [ ] `cargo check --workspace` 通过
- [ ] 启动后 10 秒睇到 `tick fired at ...` log 出现（验证时间驱动）
- [ ] 创建 50+ SparkRecord 后睇到 `⚠️ too many SparkRecord` log 出现（验证状态驱动）
- [ ] Agent 可以通过现有 tool（spawn_entity + modify_component）创建 TickAction
- [ ] Agent 可以通过现有 tool（despawn_entity）删除 TickAction
- [ ] `cargo test --workspace` 通过
- [ ] Commit 喺新 branch `feature/tick-action-step2`

---

## 9. 待讨论 / 未决

> ⚠️ 以下问题 open——下次讨论更新

1. **Rhian bridge 缺咩 function**？需要 `has_unconsumed_user_input()` / `entity_count()` / `now()` 等 DSL 帮手。
2. **cooldown_secs 嘅下限**？如果 agent 设为 0 会死循环。要唔要 runtime 强制 ≥1？
3. **TickAction 脚本抛错嘅处理**？v1：log error 继续；v2：可以自动 disable？
4. **多 TickAction 嘅调度顺序**？v1：query 顺序（不保证稳定）；v2：加 priority 字段？
5. **TickAction 嘅 nested 触发**？一个 TickAction 嘅脚本 spawn 新 TickAction，会唔会有 stack overflow？
6. **TickAction 嘅 security**？Rhai 脚本已经有 sandbox 限制，但需要确认 `define_system` / `define_tool` 喺 TickAction 入面 call 会唔会有问题。

---

## 10. v1 整体路径（已实装 + 待实装）

| Step | 内容 | 状态 |
|------|------|------|
| **Step 1** | Tick loop（reader thread + mpsc） | ✅ 已 commit |
| **Step 2** | TickAction component + 评估 | ⏳ 设计完成，尚未实装 |
| **Step 3+** | 进一步抽象（如果需要） | 💭 构思中 |

---

## 11. 引用

- Step 1 commit: `feature/tick-loop-step1` 119f84b
- 之前讨论：Sensor / Event 已 drop
- 之前讨论：Pulsing 嘅实证路径（TickAction + Rhai 实现）
- 之前讨论：Autopoiesis 雏形（agent 自主创造 TickAction）

---

**更新指南**：

- 设计修改：直接 edit 呢个文件
- 实装时：喺 §8 验证清单打勾
- 新问题：加到 §9 待讨论
- 完成后：改 status 为 "已实装"
