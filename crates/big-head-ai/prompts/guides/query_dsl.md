# query_entities 查询语法（Query DSL）

`query_entities` 接受一个字符串参数，语法如下：

## 基本结构

```
<fetch> where <filter>
```

- **fetch**：逗号分隔的组件名，指定要返回哪些组件的值
- **where**（可选）：过滤条件，只返回满足条件的实体
- 省略 `where` 时返回所有拥有指定组件的实体

## fetch 部分

```
Health, Position          # 返回 Health 和 Position 两个组件的值
*                         # 返回匹配实体的所有组件
AgentIntent               # 只返回一个组件
```

## filter 部分

条件用 `and` 或逗号 `,` 连接，支持三种类型：

### 1. 存在性过滤（has component）

```
where Enemy               # 必须有 Enemy 组件
where Enemy and Player    # 同时有 Enemy 和 Player
where WITH(Enemy) AND WITH(Player) # 同样的存在性条件
```

### 2. 排除性过滤（not has component）

```
where Enemy and !Dead     # 有 Enemy 但没有 Dead
where WITH(Enemy, Player) AND WITHOUT(Dead) # 同时有前两个组件、没有 Dead
```

### 3. 值过滤（field comparison）

```
where Score.points > 20           # Score.points 大于 20
where Score.name == 'alice'       # Score.name 等于 alice
where TickAction.script contains 'weather_query' # 字符串字段包含片段
where Position.x >= 0.0           # Position.x 大于等于 0.0
where Health.0 < 50.0             # 元组结构体用索引访问
where DynDeadline.urgent == true  # 动态组件也可以
```

**运算符**：`==` `!=` `>` `<` `>=` `<=` `contains`

**字面量**：
- 字符串：`'single quoted'` 或 `"double quoted"`
- 数字：`42` `3.14` `-1`
- 布尔：`true` `false`
- 空值：`null`

**字段路径**：用 `.` 分隔，支持嵌套 `Component.nested.field`

## 完整例子

```
Health where Enemy and !Dead                    # 活着的敌人的血量
Position, Health where Enemy and !Dead          # 活着的敌人的位置和血量
* where Player                                  # 玩家的所有组件
Score where Score.points > 20                   # 分数大于 20 的
TickAction where TickAction.script contains 'weather_query_tickaction' # 查某类脚本
Health where Enemy and Position.x < 0 and !Dead # x 负半轴、活着的敌人
```

## 注意事项

- 组件名按后缀匹配：`Health` 能匹配 `my_crate::Health`
- 值过滤的字段必须存在，否则该实体被跳过（不会报错）
- 值过滤隐含了存在性要求（不需要额外写 `where Component`）
- 动态组件（`define_component` 创建的）也支持查询和值过滤

## 组合组件，而不是维护实体名单

Archetype 是 Bevy 根据实体实际拥有的组件自动形成的组合，不需要另建表或手工维护索引。
先检查已注册的组件及其语义，复用合适的组件；缺少概念时才 `define_component`。
没有字段的标记组件可以用 `schema: {}` 定义，用值 `{}` 挂载。
`spawn_entity` 可一次挂多个组件，`modify_component` 可给已有实体添加组件，
`remove_component` 移除标记后，后续查询自然不再匹配。不要为了改变分组复制实体或删除业务记录。
给已有实体添加组件时，先确认实体身份与修改授权；组件字段必须满足 schema。

例如已定义 `DemoWork`、`DemoNeedsReview`、`DemoCompleted`：

```text
DemoWork where WITH(DemoWork) AND WITH(DemoNeedsReview) AND WITHOUT(DemoCompleted)
```

注意 fetch 列表不是 AND 条件：要同时具有多个组件，明确写在 `where` 后。
未知组件是错误，不等于“所有实体都没有它”；先检查注册情况，不要反复重试拼错的名字。

## 将相同组合变成 Lens

`define_lens_app` 的 definition 示例（这些示例组件需先注册）：

```json
{
  "id": "demo.review",
  "version": "1",
  "purpose": "观察需要复核且尚未完成的工作",
  "reads": ["DemoWork"],
  "stage": {
    "kind": "sequence",
    "stages": [
      {"kind": "archetype_with", "component": "DemoNeedsReview"},
      {"kind": "archetype_without", "component": "DemoCompleted"},
      {"kind": "project", "fields": {"title": "/DemoWork/title"}}
    ]
  }
}
```

- `sequence` 中多个存在性条件是 AND；`parallel` 是独立分支，可能重复返回同一来源，不是去重 OR。
- `reads` 决定输入范围和读取的数据：组合 Lens 所声明 reads 的并集，至少带其中一个组件的实体进入输入。
- `archetype_with/without` 只检查组件存在性，不读取该组件载荷，也不会扩展输入范围。
- `project` 改变展示形状，但保留来源的组件成员信息，所以投影后仍可筛选。
- 缓存感知被检查组件的增删；仅修改未读取标记的内部字段不会使其失效。
- 输入仍有 256 实体和字节上限；stage 筛选、`take` 在采集之后执行，不能绕开采集上限。没有全世界隐式扫描模式。
- `run_lens_app` 手动观察；启用的 `before_reasoning` Binding 在后续 cycle 观察。这不是数据变更事件订阅。
- Lens 不修改业务数据。需要持续处理时，另用有冷却和幂等检查的 TickAction/Rhai；只有需要 Agent 判断时才发 Spark。

验证方法：只建独立测试实体，观察首次匹配；挂 `DemoCompleted` 后下一次应消失；
移除该标记后应重新出现。查看执行 receipt 和 binding 状态，不把“定义成功”当作“任务执行成功”。

## 先看世界形状，再发现 Lens

`inspect_world_shape({"with":["DemoWork"],"without":["DemoCompleted"],"limit":16,"sample_limit":3})`
直接读取 Bevy archetype，返回匹配实体总数、非空 archetype 总数、组件计数和少量样本 ID。
过滤是 AND；未知组件报错。`limit` 为 1..64，`sample_limit` 为 0..8，
with/without 各最多 32 个名字。使用返回的 `next_offset` 翻页；计数覆盖全部匹配，样本不是完整名单。
组件列表及组件计数最多展示 64 项，截断会明确标记；翻页期间世界变化可能改变结果。
这个调用不序列化业务载荷，不创建持久化索引，不执行 Lens 或 Spark。

知道组件后，可随时调用 `discover_lenses({"components":["DemoWork","DemoCompleted"]})`，
不必等待下一轮推理。也可以同时给 `query`；显式组件匹配优先于文字匹配，
覆盖引用 Lens 的 reads 和存在性条件。建议不是适用性保证，查看 usage 后再运行。

## Lens 契约诊断

先用 `describe_lens_contract({})` 获取完整 schema 和示例。
检查草稿时传 `{"input":{"definition":{...},"expected_version":"1"}}`，
其中 input 是完整 define_lens_app 请求。诊断只检查定义结构、组合和版本前提，不写入、不运行，
不保证当前世界组件已注册或输入量未超限；这些由执行时检查。
诊断返回 `valid`、`errors`、`normalized_input`；有效时可将 normalized_input 交给 define_lens_app。

版本规范格式是字符串。非负整数版本可归一化为字符串，包括嵌套 Lens 引用和 expected_version。
`take.limit` 必须是整数；`kind` 不可缺省；expected_version 必须放在 definition 外，
source_updated_at_pointer 属于 Binding。诊断不会猜 stage 或自动搬移字段。
更新需完整 definition、匹配的 expected_version 和新版本；当前没有局部 patch 更新接口。
