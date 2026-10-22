# 自主触发（TickAction）

`TickAction` 用于按冷却时间或脚本条件自动执行 Rhai，无需每次由外部 prompt 触发。
它不是 runtime 唯一的自主推进机制：已启动的 ActivityRun 由活动调度器推进，不依赖 TickAction。
TickAction 的 Rust 默认值和缺省反序列化值都是 `enabled=true`，不是 false。
创建后需要等待审核的 TickAction 必须显式写 `enabled=false`；不要把“没有收到启动命令”当成已禁用的证据。

## TickAction Component

```
TickAction {
    enabled:        bool,     // 是否参与 runtime 自动调度，旧数据缺省为 true
    script:         String,  // 每次 tick 评估的 Rhai 脚本
    last_run:       u64,     // 上次执行时间（epoch secs），0 = 从未跑过
    cooldown_secs:  u64,     // 冷却时间（秒），now - last_run >= cooldown_secs 才会执行
}
```

## 创建 TickAction

当你需要设置定时或条件触发的自动行为时，使用 `spawn_entity` 创建一个
带有 `TickAction` 组件的实体：

```json
{
  "components": [
    {
      "name": "TickAction",
      "value": {
      "script": "log(\"heartbeat at \" + timestamp())",
      "enabled": true,
      "last_run": 0,
        "cooldown_secs": 60
      }
    }
  ]
}
```

## 触发条件示例

TickAction 的 script 可以表达任意观察世界的条件：

| 类型 | 示例 |
|------|------|
| **时间驱动** | `log("hourly check")` + `cooldown_secs: 3600` |
| **状态驱动** | `if entities_with("Spark").len() > 50 { log("too many") }` |
| **资源监控** | 检查 world 状态后 spawn spark 通知自己 |
| **复合条件** | 时间 + 状态组合判断 |

## 可以做的事

- Query world（`entities_with`, `has_component`, `components_of` 等）
- Spawn / despawn entity
- Modify / remove component
- Emit log
- Push spark（调用 `push_spark("...")` 创建 Spark entity + push 到 queue — 从而触发自己推理）

## TickAction 触发 LLM 的正确方式

TickAction 里不要手动 `spawn_entity(#{"Spark": ...})`。正确写法是直接调用 Rhai 内置函数
`push_spark(text)`：

```rhai
let ctx_eids = entities_with("SessionContext");
let loc = "余杭区";
if ctx_eids.len() > 0 {
    let ctx = get_component(ctx_eids[0], "SessionContext");
    loc = ctx.location;
}

push_spark("[weather_query tickaction] 重新 fetch " + loc + " 当前日实况天气，并 set_session_context 更新 weather / alerts");
log("[weather_query] queued weather refresh spark for " + loc);
```

原因：
- `push_spark` 会自动创建 `Spark` 组件并放入 runtime 的 `spark_queue`。
- 手动 `spawn_entity(#{"Spark": ...})` 容易写错 `timestamp` / `cycle_id` / `status` / `ambient_signal_ref` 的类型，并且不保证进入队列。
- TickAction 只负责轻量触发；search / fetch_url / set_session_context 应交给下一轮 Spark 的 LLM cycle。

## 显式观察状态

TickAction 可返回下面的 Map，供 runtime 判断语义状态是否改变：

```rhai
let shape = inspect_world_shape(#{with:["Topic"], without:["TopicInsight"], sample_limit:0});
let count = shape.matched_entities;
log("orphan topics=" + count.to_string());
return #{tick_observation:#{
    state_key:"orphans=" + count.to_string(),
    summary:"Topics without insights: " + count.to_string(),
    urgent:false
}};
```

示例只关心数量；若关心成员替换或内容变化，状态键必须包含对应的稳定证据。
不要把时间戳、随机 ID 或无关总数放入 `state_key`。
`state_key` 必填且非空，最多 1024 字节；`summary` 最多 4096 字节；
`urgent` 默认 false。整个 `tick_observation` 最多 8 KiB，未知字段或错误类型会形成执行错误。

首次观察、状态键或脚本改变、新 Spark 都会入观察队列；普通相同状态每累计 12 次重复再报告一次。
执行错误和 `urgent:true` 每次都绕过重复抑制，但仍受有界队列与处理重试机制约束。
日志和 summary 改变不会覆盖显式状态键；不返回状态的旧脚本继续使用日志归一化路径。
状态是脚本声明，不是业务完成回执，也不会替你保证写入幂等。

## 生命周期规则

- **创建**：spawn_entity 挂 TickAction → 下个 tick 开始评估
- **修改**：modify_component 改 script/cooldown_secs → 即时生效
- **删除**：despawn_entity → 立即停止
- **暂停**：update_tick_action 设置 `enabled=false` → 保留脚本、实体和运行状态，但不再自动评估
- **恢复**：update_tick_action 设置 `enabled=true` → 按现有 `last_run` 和 `cooldown_secs` 重新参与调度
- **last_run**：runtime 在脚本执行后统一写回 `last_run = now`，脚本本身不要再手动维护
  `last_run`。如果你看到 `last_run` 没变，优先检查该 TickAction 是否真的被 cooldown 选中执行、
  当前二进制是否已重启到最新版本、以及 checkpoint 是否来自旧版本。
- **落盘时机**：TickAction 执行后标记待保存；即使后续没有报告，主循环也会检查到期补存。
  默认距上次成功保存至少 900 秒，可用 `BIGHEAD_TICK_CHECKPOINT_SECS` 覆盖，`0` 表示不节流。
  首次待保存立即尝试；任一完整保存成功都会清除待保存标记。失败保留修改并退避 30 秒。
  observer/Spark 的关键投递保存、交互结束和正常退出仍保留，因此不是全局每 900 秒只写一次。
  后台间隔使用单调时钟；长时间阻塞、写失败会延长实际落盘窗口，不能承诺崩溃最多丢 15 分钟。
  新 checkpoint 将大于 4 KiB 的 ShardData 正文放在同名 `.blobs` 目录，运行时仍能读取完整正文。
  observer/Spark 关键保存通过带校验的 `.journal` 增量日志即时落盘，定期合并为完整主文件。
  备份或移动 checkpoint 必须同时保留 `.blobs` 和 `.journal`；旧内联 checkpoint 可直接恢复，新格式不要用旧二进制打开。

## 注意

- TickAction script **不能直接调用 LLM**，必须通过 `push_spark("...")` 间接触发推理
- 创建或修改 `TickAction` 时，`last_run` / `cooldown_secs` 必须是 JSON 数字，例如 `0`、`21600`，不要写成 `"0"`、`"21600"`
- `enabled` 必须是 JSON boolean；旧 checkpoint 缺少该字段时 runtime 按 `true` 恢复
- 脚本在每个 tick 都是新 scope，不保留局部变量
- cooldown_secs 不要设为 0，避免死循环
