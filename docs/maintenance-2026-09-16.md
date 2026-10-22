# Runtime Composition Follow-up

Continues the uncommitted follow-up to `bc430c8` from session
`019ee285-df99-7271-8a4f-5a74c6a3e4a6`.

## Delivered

- Runtime Rhai calls the same Lens definition, execution, binding, discovery,
  contract diagnosis and world shape services as tools, without nested evaluation.
- Failed Rhai mutations raise execution errors. Failed entity creation removes
  that new entity; previous successful script mutations are not rolled back.
- Entity resolution checks spawned status and rejects IDs outside u32 range.
  Bevy `Entities::contains` also accepts unspawned handles and was insufficient:
  removing a dynamic component through Rhai with ID 999999 could panic.
- Lens tools expose recursive stage schemas and read-only contract diagnosis.
  Nonnegative integer versions normalize to strings; other malformed fields
  remain errors. Diagnosis checks structure and catalog preconditions, not live
  input availability or execution budgets.
- Explicit component discovery ranks reads and presence dependencies, including
  referenced Lenses, ahead of lexical matches.
- World shape inspection reads Bevy archetypes for exact matching counts and
  bounded pages/samples. Lens collection skips unrelated archetypes while
  preserving the union of reads, membership evidence, stable identities and
  the 256-source limit. Stage filters still run after collection.
- TickAction can return `tick_observation` with `state_key`, `summary`, `urgent`.
  Stable keys suppress log churn; errors, urgent reports and new Sparks bypass
  repeat suppression. Legacy scripts keep log normalization, including numeric
  archetype snapshot timestamps. Reports reach session logs and the TUI.
- Built-in Query, Rhai and TickAction guides explain these contracts. Roadmap
  Stage 1.2 uses native archetype access instead of a separate membership list;
  source/provenance and shard indexes remain future work.

## Validation

```sh
cargo test -p big-head-ecs -p big-head-loop -p big-head-script -p big-head-world -p big-head-query -p big-head --offline
cargo test -p big-head-loop --offline measure_archetype_collection_16600_entities -- --ignored --nocapture
git diff --check
```

Regression result: 315 passed, 11 ignored (9 unit tests and 2 doc examples),
zero failures. The separately invoked synthetic measurement also passed.
Existing vendor cfg and unrelated dead-code warnings remain.

Synthetic debug-build measurement: 16,600 entities, 200 matching source facts,
50 iterations, after warm-up. The old and new serialized facts were equal.

| Operation | Mean time |
| --- | --- |
| Legacy entity scan and serialization | 4.378 ms |
| Archetype collection and serialization | 2.247 ms |
| Filtered world shape inspection | 0.042 ms |

This is a local synthetic measurement, not a production benchmark or a token
cost forecast. No live model, checkpoint-size benchmark or online script
migration was performed.

## Human Verification

Use an isolated test world after rebuilding and restarting the binary. Run the
built-in `guide_query_dsl`, `guide_rhai_syntax` and `guide_tick_action` guides
(check available guide IDs with the system registry if customized).

1. Define a data component and an empty completion marker, then create one test
   entity. Inspect world shape with the data component and without the marker.
2. Diagnose and publish a Lens with the same reads and presence condition.
   Discover it by component, then execute it through Rhai and the tool.
3. Add/remove the completion marker; Query and Lens should both lose/regain the
   entity. Shape counts should follow; samples are not a full entity list.
4. In a test TickAction return a stable state key while varying logs. Ordinary
   repeats should be suppressed until 12 repeats; change the key to emit again.
   An execution error or urgent declaration should bypass repeat suppression.
5. Attempt a Rhai mutation on an absent entity. Expect an execution error, not
   a process panic; do not infer rollback of earlier successful writes.

## Boundaries

No live business entities or TickAction scripts were modified. Rust changes and
embedded guide updates require rebuilding and restarting. Existing scripts only
use explicit state keys after they are updated to return them. Observation
queues remain bounded with retry backoff; bypassing suppression is not an
unlimited delivery guarantee. Event subscriptions and partial Lens patch updates
are not part of this change.

## Suggested Commit Message

```text
feat(runtime): 贯通 Rhai 与 Lens，补齐世界形状查询和观察收敛

- Rhai 复用 Lens 工具服务，写入失败显式报错，失败创建撤销新实体
- 修复未生成实体误判有效及超范围 ID 截断，避免动态组件操作 panic
- 提供完整 Lens stage schema、只读契约诊断及整数版本归一化
- 按组件发现 Lens，覆盖引用链的读取和成员依赖
- 基于 Bevy archetype 返回计数与有界样本，优化 Lens 输入采集
- TickAction 支持显式状态键，错误和紧急观察绕过重复抑制
- 补齐内置指南、路线图边界与回归/合成性能验证记录

验证：315 项回归通过、11 项忽略；另行通过 16600 实体合成测量。
边界：未迁移在线脚本；需要构建并重启；不提供整段脚本事务回滚。
```
