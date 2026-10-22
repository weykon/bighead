# Lens 的发现、反馈与候选演进

这一版让现有操作 agent 能把反复探索或用户纠正转成可复用的观察方式。不是一个常驻学习 agent，也没有为每个请求额外调用规划模型。

## 运行路径

```text
Spark -> 已绑定 Lens 的本轮观察
      -> 任务相关能力建议（只读元数据，不执行 Lens）
      -> 原有 agent tool loop
         -> 有合适观察：复用
         -> 缺少信息：发现并手动运行相关 Lens
         -> 实际缺口/用户纠正：反馈 -> 候选 -> 试验 -> 采用
      -> 下一任务可发现已采用的 Lens
```

建议只在工具任务首次 reasoning 前注入，不随每个 wave 重算。最多 3 条建议，建议 JSON 最多 4 KiB；工作流引导另有固定小段文字。检索使用英文词和中文双字片段的词面交集，不是 embedding 或可靠的语义选择器。没有匹配时可调用 discover_lenses 换用更具体的词，或 inspect_lens_apps 查看已有定义。

## 定义的适用信息

LensDefinition 新增可选 usage，旧定义默认空列表，旧 checkpoint 无需迁移：

```json
{
  "usage": {
    "tasks": ["检查天气刷新任务的最近执行时间"],
    "tags": ["天气", "weather", "调度"],
    "limitations": ["任务运行不证明外部天气数据已经更新"]
  }
}
```

每个列表最多 8 项，每项 256 bytes。检索只正向匹配 id、purpose、tasks、tags、reads，不因 limitations 中出现某个词而推荐一个不支持该任务的 Lens。

## 工具与持久化

- discover_lenses：按任务检索已发布定义，附带最近评估，候选不混入结果。评估明确标为 agent assessment，而非验证事实。
- record_lens_feedback：保存 useful/gap/correction、说明、Lens 版本和 runtime 关联的 Spark 来源。相同来源/版本/内容/类型重复提交复用原记录。最多 256 条，满时显式归档清理，不静默删除。
- propose_lens_candidate：定义必须使用新的 Lens id，可固定 based_on={id,version} 和最多 8 条对应版本的反馈 id。修改未采用候选需 expected_revision，会使旧试验失效。
- trial_lens_candidate：试运行并记录最新试验。checks 默认 min_facts=1、required_fields=[]、max_output_bytes=2048；required_fields 为每条输出 value 下的 JSON Pointer，缺失或 null 会失败。
- adopt_lens_candidate：要求最新通过试验的 trial_id 和采用理由，重新运行检查并比较源数据/目录变化标记；变化时拒绝采用，要求重新试验和审阅。
- inspect_lens_learning：检查候选状态、原因、反馈及试验；details=true 必须指定 candidate_id，避免返回大量试验正文。

LensFeedback 是 agent 通过专用工具写下的评估。LensCandidate 存定义、基线、原因、反馈引用、revision、status 和最新试验。状态为 proposed/passed/failed/adopted，最多 32 个候选（包含已采用记录）。每个候选保留最新试验而非无界历史，回执最多 512 KiB；均随 checkpoint 恢复，不自动采用或激活。

## 试验与采用边界

基线和候选使用同一次采集的输入，再各自按声明的 reads 裁剪，不能因为候选读更多组件而把那些数据泄露进基线的 identity 输出。
对比项是输出条数、序列化字节数、null 数量；检查必需字段和最低条数。零输出不算成功。试验的采集/执行错误也落失败状态，不能仅靠 agent 总结写 passed。

结构检查不验证业务正确性。更小的上下文不一定更好，运行时间不是天气更新时间，来源内容是否新鲜仍需领域事实和用户反馈。
采用前的 evidence_key 沿用运行时的非密码学内容摘要，作为变化检测而非防篡改或授权凭证。源内容、候选正文或已发布目录变化均要求新试验。
试验输出字节预算不是绑定的 2 KiB 注入预算：绑定还包含额外元数据，采用后需要持续观察时应验证实际绑定结果。

采用把候选发布为独立 Lens；旧定义、手动回执、绑定、启停和优先级完全不动。同一次已成功采用的重试，在发布正文仍一致时幂等返回。
没有自动创建 TickAction、push_spark 或外部调用，没有 Lens 内任意脚本阶段。切换绑定仍走 define_lens_binding 的显式 revision 检查。旧 define_lens_app 保持兼容；候选流程是更稳妥的演进路径，不是对通用 ECS 写工具新增的安全隔离层。

## 自然语言联调

重启到新代码后，可以向内部 agent 说：

> 检查现有天气任务时，先找能复用的观察方式。当前我不需要整段脚本，只需要实际能证明的执行状态。若现有 Lens 提供了过多无关信息，记录这个缺口，形成一个更紧凑的候选并试运行。明确哪些结论仍不能证明，采用适合的观察方式，但不要修改现有绑定或业务数据，也不要排新 Spark。

验收应看真实工具轨迹与 inspect_lens_learning：是否引用了存在的基线/反馈；候选是否在试验前保持隔离；试验是否保存真实输出和指标；采用后旧绑定是否不变。模型回答“已进化”本身不构成证据。

尚未实现：独立的语义规划 agent、自动效果评分/最优 Lens 选择、跨任务语义聚类、后台主动学习、任意历史回放、自动绑定迁移或图形编辑界面。本版提供 agent 主导的有界流程，真实模型是否稳定使用仍需联调评估。
