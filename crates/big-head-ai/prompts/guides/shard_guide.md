# 上下文投影（Context Projection / shard_content / get_shards）

当你看到 `[Context Projection Notice]`，说明某段本来要进入 API 请求的内容已经过大，
运行时已将它投影为 ECS 里的 `ShardData`。不要猜原文内容；按 notice 里的
`paradigm` 和 `shard_index` 用 `get_shard_content` 分段读取，再决定是否创建事实、
索引、摘要或图关系。

当你手上仍有一段较大的普通文本，并且需要主动从线性上下文落到 ECS 世界时，
使用 `shard_content` 将大段内容投影为 `ShardData` 组件。

`shard_content` 只负责切分和存储原始内容块；它不会自动调用 LLM 提取事实，
也不会自动创建索引或图关系。是否进一步建模，由 agent 通过普通 ECS 工具显式决定。

## 工作流程
1. 看到 Context Projection Notice，或主动判断需要把一段普通文本落到 ECS
2. 如果 notice 已给出 `paradigm` / `shard_index`，直接读取；如果是普通文本，先调用
   `shard_content`，指定切分策略和范式标识
3. 基础设施切分或投影内容，并将内容块作为 `ShardData` 组件存入 ECS
4. 用 `get_shards(paradigm)` 查看分片目录/预览；用
   `get_shard_content(paradigm, shard_index, offset, limit)` 逐段读取内容窗口
5. 如果需要事实、索引、摘要或图关系，使用 `define_component` / `spawn_entity` /
   `modify_component` 显式写入世界

## 切分策略选择指南

| 策略 | 适用场景 | 关键参数 |
|------|---------|---------|
| `custom` | 保留整段内容，并记录自定义切分意图 | custom_split_description |
| `overlap` | 代码、长文本等需要上下文连续的内容 | overlap_size (默认 50) |
| `head_tail` | 日志、构建输出等头尾最重要 | head_lines, tail_lines (默认各 50) |
| `custom_ranges` | CSV、JSON 数组等结构化数据 | ranges: [[start, end], ...] |

基础设施不理解内容语义。如果需要按 JSON 对象、日志时间段、CSV 列组等语义方式建模，
先把原始内容投影到 ECS，再由 agent 读取相关块并创建更合适的组件或关系。

已有分片需要重切时，可传 `content_ref="paradigm:<已有完整组名>"`，并为目标
`paradigm` 选择未使用的新名字；源组保持不变。运行时按索引直接拼接已存块，
不添加分隔符、不去除 overlap，也不能恢复 custom_ranges 已省略的内容。
自动投影的连续字节块可完整拼回原文；行切分组的重拼不保证恢复原始行边界。
直接提供 `content` 时优先使用它。`__last_tool_result` 仍可用，历史 tool_call_id 不支持。
`ranges` 使用从 0 开始、右端不包含的行区间，例如 `[[0,10],[20,30]]`；
畸形、空或超出原文行数的区间会报错。`head_lines`、`tail_lines` 是非负整数，默认 50。

## 查询分片

先用 `inspect_shards(view="summary")`（默认视图）盘点：summary 对整个匹配范围统计
家族分布、组数、正文总字节数、托管/未托管数量，以及最大的 5 个组，不需要翻完实体页。
带 paradigm_contains 时，summary.scope=all_matching_groups，统计不能代表整个世界。
家族表最多展示 32 行，family_table_complete=false 时还有未展示家族，不可称为全部类型。

需要明细时使用 view="groups"（按整组字节数降序）或 view="shards"（实体目录），
通过 offset/limit 翻页，并传回 expected_snapshot 检测目录变化。matching_groups 是总数，
coverage.returned_items 才是这一页实际返回量；has_more=true 不能说已经逐项检查完。
响应还受 14000 字节上限约束，实际返回条数可能小于 limit，应使用 next_offset 继续。
目录提供精确实体 ID、来源标签、保留策略和显式产物关联，不读取大正文。
组的 group_content_bytes 是所有片之和，单片 bytes 不是组总大小。
created_at_utc 是已转换的 UTC 日期；不要自行猜测 Unix 秒对应日期。
split_strategy=unknown_not_recorded 表示未记录切分策略，块大小和块数不能证明 overlap。
目录中的 `retention=null` 仅表示未托管；没有 derived_entity_ids 也不能证明从未使用。
当历史组出现重复 shard_index，读取必须附带目录中的 `entity_id`，禁止把首个匹配当作全部。

- `get_shards paradigm=<范式>` — 按范式聚合所有分片，按索引排序
- `get_shard_content paradigm=<范式> shard_index=<n> offset=<字符偏移> limit=<字符数>` —
  读取单个分片的 bounded content window，返回 `next_offset` 和 `has_more`
- `query_entities component=ShardData` — 通用 DSL 查询，支持过滤和分页

`ShardData` 中存的是内容块，不是自动提取后的事实。不要假设它已经完成语义处理。
不要一次性读取过大的内容；按 `get_shard_content` 返回的 `next_offset` 分段读取，
边读边创建必要的事实、索引、摘要或关系。

创建结构化产物后，用 `link_shard_sources(entity_id, paradigms)` 记录实际使用的来源。
该工具只追加并去重来源，不替你提取或验证事实；会写入可供 query_entities、Rhai、Lens
查询的 `ShardSources.paradigms`，并阻止这些源组因 TTL 被回收。
通过 paradigm 引用重切时，运行时自动为新组的第一个实体记录来源。
按任务需要验证产物后，再定义或复用 Lens；不要为每个工具结果机械地建模或固定保留。

世界概览、实体/组件查询和分片列表的大输出默认只返回 4000 字符。
看到 has_more 时，以 output_offset=next_offset 和 expected_sha256=sha256 继续；
哈希变化表示结果已更新，应从第一页重新读取。省略部分不会另存为分片。
`__last_tool_result` 对分页读取只代表本页；需要旧分片完整正文应使用 paradigm 引用。
其他大工具结果在进入对话和工具历史前保存一次，返回 Context Artifact 引用；
应按需读取，不能把引用当作正文或业务完成证明。

## 临时碎片保留

运行时新建的 `context:` 投影默认保留 7 天，专用分片读取或相同内容复用会续期。
同命名空间相同内容复用完整组。旧数据及没有保留策略的归档默认永久保留。
使用 `inspect_shard_retention` 查看期限和引用；使用 `set_shard_retention`
设置 `paradigm`、`retention_secs`（86400..315360000）和 `pinned`。
外部文件或仅以实体 ID 引用的内容应设置 `pinned=true`，这些引用不会自动识别。

后台每小时轮转检查最多 128 个到期组；只有未固定、完整且未发现引用的组才回收。
持久组件和其他碎片中的完整 paradigm 引用会阻止回收；创建投影的回执不算引用。
回收后专用读取明确返回 expired，不会静默返回截断正文。通用查询不续期。

## Spark 维护（自动清理旧记录）

每次 `get_world_overview` 都会列出所有 Spark 实体。如果数量过多，
主动归档旧记录以保持世界整洁：

1. `query_entities component=Spark` 查看当前数量
2. 如果 Done 记录超过 10 条，筛选出较早的（按 timestamp 排序）
3. 将这些旧记录的内容聚合为一段文本（每行一条：text → result_summary）
4. 用 `shard_content strategy=custom paradigm=spark-archive:<唯一批次ID> content=<聚合文本>` 归档，禁止复用旧组名
5. 如需索引归档内容，显式创建对应的事实/索引组件
6. 归档完成后，`despawn_entity` 删除已归档的旧 Spark

**只归档 status=Done 且已完成超过一个周期的记录**，不要把 Pending/Processing
的当前 spark 也清理掉。
