# Ambient Dwelling 落实文档 (MVP 1.0 + 2.0)

**Document ID**: `doc-ambient-dwelling-mvp-spec-001`
**Topic**: bighead-gameworld-001 (Insight 10 Ambient Dwelling)
**Author**: user + agent co-creation (2026-08-09)
**Status**: ready-for-implementation
**Target Agent**: bighead / harness implementer (any crate 都可以参与)
**Source Entity**: E495 BizModelingDoc (extracted 2026-08-09 18:00 UTC+8)

---

## 🎯 目标

由「介面 (artifact)」到「场 (field)」嘅跃迁。Bighead 设计 layer 由「做 UI」扩展到「design for ambient co-existence」:
- **空间感** —— agent 感受到身处嘅环境 (公园 / 房间 / 书房)
- **感知场** —— push-based signals (而非 pull-based queries)
- **共处层** —— user + agent 共享同一个 ambient space (MVP 3.0, 暂不实现)

呢份文档聚焦 **MVP 1.0 + 2.0** —— 最小可行嘅 ambient primitive。

---

## 🌱 MVP 1.0 — Ambient Cue (静态开场)

### 概念

每次 agent wake-up / spark push, world 主动 attach 一段**环境描述**到 cycle 开场白。Agent 唔需要 query, 系被动接收。

### ECS 改动

#### 新 component: `SessionContext`

```rust
// crates/big-head-ecs/src/components/ambient.rs (建议)
#[derive(Component, Reflect, Serialize, Deserialize)]
pub struct SessionContext {
    pub location: String,           // 公园 / 房间 / 书房 / 海边
    pub weather: String,            // 雨后 / 黄昏 / 朝早
    pub time_of_day: String,        // 黎明 / 上午 / 下午 / 黄昏 / 夜晚
    pub light: String,              // golden_hour / overcast / 夜灯
    pub updated_at: String,         // ISO timestamp
    pub source: String,             // user_manual / auto_detected / preset
}
```

#### Entity 形式 (建议)

- 一个**单例 entity** (类似 IsResource), 持有 `SessionContext` component
- 或者 per-session entity (如果有 multi-session 概念)

#### Tool API

新工具: `set_session_context`
- **LLM 可见 tool** (注册 via `define_tool`)
- 接收 `location` / `weather` / `time_of_day` / `light` 四个字段
- 写入 `SessionContext` component
- 唔需要 agent 调用, **user 手动设置或者周期 tick 自动**

#### Cycle 接入

在 `get_world_overview` 之前 (即系每个 agent cycle 嘅最开头), 引擎:
1. Read `SessionContext` 单例 entity
2. 如果存在, prepend 一段到 cycle 开场白:

```
[Ambient Cue]
你身处: 书房
天气: 雨后
时间: 黄昏
光线: golden_hour

[System Overview]
... (原有 world overview)
```

3. 如果唔存在, 跳过 (MVP 1.0 阶段 ambient 系 optional)

### 验收标准

- [ ] `SessionContext` component schema 注册成功
- [ ] `set_session_context` tool 注册到 LLM tool list
- [ ] Cycle 开场白包含 ambient cue 段落 (when present)
- [ ] Agent 嘅响应会自然地 reference ambient cue (e.g. 「傍晚嘅书房好安静」)
- [ ] SessionContext 唔存在时, cycle 行为唔受影响

### 隐私边界 (MVP 1.0)

- SessionContext **纯 manual / preset**, 冇 auto-detect
- User 完全控制 ambient 嘅内容, 唔会 leak 个人 detail
- Agent 只可以 read, 唔可以 modify SessionContext

---

## 🌿 MVP 2.0 — Ambient Event (显式事件)

### 概念

每次 spark push 携带 `AmbientSignal` JSON, 由 event source 触发 (user action / world tick / external trigger), 唔系**周期注入**。

### ECS 改动

#### 新 component: `AmbientSignal`

```rust
#[derive(Component, Reflect, Serialize, Deserialize)]
pub struct AmbientSignal {
    pub signal_id: String,          // uuid
    pub category: String,           // natural / human / technical / emotional
    pub wind: String,               // 静 / 微风 / 大风
    pub birds: String,              // 安静 / 雀仔声 / 蝉鸣
    pub light: String,              // golden_hour / 阴天 / 夜灯
    pub temperature: String,        // 凉 / 暖 / 热
    pub intensity: f32,             // 0.0 - 1.0
    pub source: String,             // user_action / world_tick / external
    pub timestamp: String,          // ISO
    pub ttl_secs: u32,              // signal 存活时间 (default 300)
}
```

#### Entity 形式

- Transient entity, 每次 signal spawn 一个
- TTL 到期自动 despawn (via existing tick_action infra)

#### Spark 接入

修改 `Spark` component payload (或者扩展) 加入 `ambient_signal_ref: Option<String>`, 指向一个 AmbientSignal entity id。

或者更轻量: 直接将 AmbientSignal 字段 flatten 入 Spark 嘅 metadata。

### Tool API

新工具: `push_ambient_signal`
- 接收 signal 字段
- spawn AmbientSignal entity, 自动 attach 到下一个 spark
- 触发 agent 响应 (push a new spark)

#### 触发 rules (event-driven, 非周期)

1. **User action**: user 切换 location / 完成 task, 触发 ambient signal
2. **World tick**: 大时间跨度 (e.g. 黄昏 → 夜晚), 自动 signal
3. **External trigger**: 外部传感器 (将来 MVP 3.0), privacy-sanitized

### 验收标准

- [ ] `AmbientSignal` component schema 注册成功
- [ ] `push_ambient_signal` tool 注册到 LLM tool list
- [ ] Agent 嘅响应会 reference ambient signal 嘅具体内容
- [ ] Signal TTL 到期自动 despawn
- [ ] 同 Spark system 正确桥接 (signal 随 spark 一齐到 agent)

### 隐私边界 (MVP 2.0)

- `category=human` 嘅 signal 只可以 sensing **atmosphere**:
  - ✅ 声音强度 (静 / 嘈)
  - ✅ 动作频率 (慢 / 快)
  - ❌ 键盘内容 / 语音内容 / 屏幕内容 (永远唔可以)
- User 可以**永久 disable** ambient sensing via setting
- 所有 ambient signal 写入 ECS 之前, 过一个 `PrivacyFilter` (未来 hook)

---

## 🛠 ECS 整洁度改进 (同步实施)

Insight 10 嘅 6.5/10 cleanliness reflection 指出以下**必须同步改进**, 唔可以留低 tech debt:

### 1. Glossary component (新)

```rust
#[derive(Component, Reflect, Serialize, Deserialize)]
pub struct Glossary {
    pub term: String,               // "ambient"
    pub definition: String,         // 完整解释
    pub category: String,           // design / technical / meta
    pub first_introduced_at: String,
    pub related_insight_ids: String, // CSV
}
```

Seed data (MVP 1.0 同步):
- `ambient` —— design term, ambient cue / event / co-presence 三层
- `commensal co-existence` —— AI ECS 共生 metaphor
- `cross-axis bridge` —— relation type
- `crystallize` —— insight lifecycle
- `sparse but meaningful` —— ambient principle

### 2. InsightStatus enum (硬约束)

由 free string → enum:
```rust
#[derive(Component, Reflect, Serialize, Deserialize)]
pub struct InsightStatus {
    pub status: String,             // "open" | "crystallizing" | "done" | "deprecated"
}
```
或者直接 modify `TopicInsight.status` field 加 enum constraint via validate hook。

### 3. WorkItem.paradigm_id 字段

WorkItem component 加 `paradigm_id: String`, 强制 partition:
- `paradigm_id=bighead_main` (大主线)
- `paradigm_id=gameworld_vision` (愿景)
- `paradigm_id=external_favor` (林总机械厂 etc.)
- `paradigm_id=vue_learning`
- `paradigm_id=refactor`

### 4. Naming convention 文档化

```
<paradigm>-<concept>-<subconcept>
例: bighead-gameworld-ambient-mvp1
例: bighead-gameworld-ecs-coexistence
```

---

## 📋 Sprint Plan (建议)

### Sprint 1 (MVP 1.0 落地)
1. `SessionContext` component schema define
2. `set_session_context` tool define (via `define_tool`)
3. Cycle 开场白接入 ambient cue 段落
4. 单元测试 + manual test
5. 同步: Glossary seed data (5 个 term)

### Sprint 2 (MVP 2.0 落地)
1. `AmbientSignal` component schema define
2. `push_ambient_signal` tool define
3. Spark system 桥接 AmbientSignal
4. TTL auto-despawn hook
5. 同步: InsightStatus enum + WorkItem.paradigm_id 字段

### Sprint 3 (cleanliness 收尾)
1. Naming convention 文档化 (CLAUDE.md / 指南 system)
2. 现有 insight / workitem retroactively rename (optional)
3. WorkItem partition 强制 check

### Backlog (MVP 3.0)
- Ambient Co-presence (真实传感器接入)
- Privacy contract 完整设计
- Agent 同 user 共享 ambient space 嘅 protocol

---

## 🔗 相关 entity 索引

- **E407 Topic**: bighead-gameworld-001
- **E485 TopicInsight** (新 spawn): `insight-ambient-dwelling`
- **E487 TopicRelation**: Insight 10 ↔ E310 (cross-axis time × ambient)
- **E489 TopicRelation**: Insight 10 → E349 介面诗学 (extends)
- **E491 TopicRelation**: Insight 10 → E312 生活引擎 (informs)
- **E493 TopicRelation**: Insight 10 ↔ E416 AI ECS 共生 (complements)
- **E407.why_it_matters**: 已经提到 Insight 10

---

## 📞 交接 checklist (俾 implementer)

- [ ] 读 E485 (Insight 10 body) — 完整设计 rationale
- [ ] 读 E407 (Topic why_it_matters) — context
- [ ] 读 E416 (Insight 9 AI ECS 共生) — 持续身份 relation
- [ ] 睇 `crates/big-head-ecs/src/components/` 现有 component 命名风格
- [ ] 用 `define_component` 注册 SessionContext / AmbientSignal
- [ ] 用 `define_tool` 注册 set_session_context / push_ambient_signal
- [ ] 修改 cycle 开场白接入逻辑 (建议改 `crates/big-head-loop/`)
- [ ] 同步落地 Glossary / InsightStatus enum / WorkItem.paradigm_id

---

## 🌊 设计哲学 (必读)

**「Sparse but meaningful」** —— Ambient signal 唔可以 push-based 嘅 spam, 要**重大 moment 转 ambient**:
- User 切换 location ✅ push
- 时间过咗大段 (e.g. 黄昏 → 夜晚) ✅ push
- 每个 tool call ❌ 唔 push
- Agent 完成 task ❌ 唔 push (unless task nature 同 ambient 相关)

呢个 rule 系 ambient 同 intrusive monitoring 嘅分界线。

---

**End of Document** 🌀
