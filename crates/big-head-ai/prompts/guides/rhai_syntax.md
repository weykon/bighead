# Rhai 脚本系统

你可以使用 `define_system` 注册一段 Rhai 脚本，然后用 `run_system` 执行它。
Rhai 脚本可以直接操作 ECS 世界——查询实体、读写组件、生成实体等。

**重要：Rhai 是 Rust 风格的脚本语言，不是 JavaScript/Python。** 如果你不熟悉 Rhai，
写脚本前先参考下面的语法规则。

## 工作流程
1. `define_system` — 注册脚本（先验证语法，不执行）。语法错误会返回具体行号和位置。
2. 如果 define_system 失败，仔细阅读错误信息，修正语法后重试。**最多重试 2 次**，如果仍然失败，
   改用逐个调用 modify_component / query_entities 等工具完成同样的任务。
3. `run_system` — 执行已注册的脚本，返回 log() 输出。

## Rhai 语法速查（必读）

**基本语法（类 Rust，不是 JS/Python）：**
```
// 每个语句必须以分号结尾（必须！最常见的错误就是漏分号）
let x = 42;                    // 变量声明
let s = "hello";               // 字符串
let flag = true;               // 布尔值
x = x + 1;                     // 重新赋值（不需要 let）

// 数组
let ids = entities_with("Task");
let count = ids.len();         // .len() 不是 .length
let first = ids[0];            // 索引访问

// 字符串拼接
let msg = "Found " + count.to_string() + " tasks";
//         注意：数字必须 .to_string() 才能拼接到字符串

// 条件
if x > 0 {
    log("positive");
} else if x == 0 {
    log("zero");
} else {
    log("negative");
}

// 循环
for id in ids {
    let comp = get_component(id, "Task");
    log(comp.title);
}
// 没有 C 风格的 for(;;) 循环

// Map 访问
let t = get_component(entity_id, "Task");
log(t.title);                  // 点号访问
log(t["title"]);               // 或者字符串索引
```

**完整示例 —— 统计并输出高优先级任务：**
```rhai
let tasks = entities_with("Task");
let count = tasks.len();
log("Total tasks: " + count.to_string());

for task in tasks {
    let t = get_component(task, "Task");
    if t.priority == "high" {
        log("HIGH: " + t.title + " (due: " + t.due_date + ")");
    }
}
log("Done.");
```

## 可用函数
Rhai 脚本中可以直接使用以下函数：
- `all_entities()` → 返回所有实体 ID 列表
- `entities_with(name)` → 返回包含指定组件的实体 ID 列表
- `has_component(eid, name)` → 实体是否拥有指定组件
- `get_component(eid, name)` → 获取组件值（返回 Map）
- `set_component(eid, name, value)` → 更新组件（patch 合并）
- `remove_component(eid, name)` → 移除组件
- `components_of(eid)` → 返回实体拥有的组件名列表
- `query(dsl)` → 使用查询 DSL 查询，返回结果列表
- `spawn_entity(components)` → 生成新实体，返回 ID
- `despawn(eid)` → 删除实体
- `log(msg)` → 输出日志（run_system 会捕获并返回）
- `timestamp()` → 当前时间戳

## 适用场景
- 批量操作：需要遍历所有实体并修改的逻辑（比逐个调用 modify_component 高效）
- 条件逻辑：if/else、循环等复杂处理
- 数据聚合：统计、汇总组件数据后用 log() 输出结果
- 自定义查询：用 Rhai 脚本组合多次 query 调用

## Runtime Lens 服务

Runtime 中的 Rhai 可以直接调用以下函数，参数为一个 Map，字段与同名工具相同，返回 Map：
`define_lens_app`、`run_lens_app`、`inspect_lens_apps`、`describe_lens_contract`、
`define_lens_binding`、`inspect_lens_bindings`、`discover_lenses`、`inspect_world_shape`。
这些函数不调用 LLM，也不嵌套执行 Rhai。

```rhai
let shape = inspect_world_shape(#{with:["Topic"], without:["TopicInsight"], sample_limit:2});
let suggestions = discover_lenses(#{components:["Topic", "TopicInsight"]});
log(shape.matched_entities.to_string());
```

组件需先注册。用 `define_component("TopicInsight", #{})` 定义空标记；
用 `set_component(eid, "TopicInsight", #{})` 挂载，`remove_component` 移除。
Bevy 自动形成 archetype，不需要再注册一份实体名单。
复用命名 Lens 时使用 `#{kind:"lens", id:"existing.id", version:"1"}` 阶段。

写入、定义及查询失败会抛出 Rhai 错误，未捕获时终止脚本；
`get_component` 读取缺失值仍返回 `()`。失败的 `spawn_entity` 撤销该次新建实体，
但整个脚本不是事务：出错前其他成功的写入仍存在，重试必须检查现状。
