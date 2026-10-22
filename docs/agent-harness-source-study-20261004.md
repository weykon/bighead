# 开源 Agent 执行框架对照与吸收方案

日期：2026-10-04。范围：固定提交的源码与测试阅读，结合 Big Head 当前工作树对照；不是三个项目的完整审计，也没有运行它们的测试。本文提出的改造尚未安装到 Big Head runtime。

## 结论

我们应当借鉴的是可靠执行、上下文装配、来源追踪和可重复验证的方法。ECS 继续存储事实与动态业务概念，Lens/Frame 继续提供解释视角，agent 继续可以修改被授权的流程。Rust 核心负责通用执行约束，不需要内置天气、项目管理等业务模型。

之前的“部分通过”不能理解为成功率 50%。记录写入、引用有效、流程边界通过，不等于自然理解、知识正确性、观察准确性同时通过。应分别报告这些维度，参见 `a2a-daily-validation-20261003.md`。

## 获取的源码

本地原有 `~/Desktop/p/pi-mono`（2026-01-30 提交）、`~/Desktop/p/opencode`（2026-02-12 提交），均保留未动。另取官方 GitHub 固定提交归档，解压至 `~/Desktop/p/agent-reference-sources/20261004/`，约 275 MB；这是源码快照，不含 Git 历史，没有安装依赖。

| 项目 | 提交 | 快照目录 |
| --- | --- | --- |
| Pi | `200387122ca450d6387f033949423114a270b96c` | `pi-200387122ca450d6387f033949423114a270b96c` |
| Codex | `ab45264919aaeb8a421cc156f1a5459ef9d60b72` | `codex-ab45264919aaeb8a421cc156f1a5459ef9d60b72` |
| OpenCode | `907b3bc518fa48e90e8ec24dd327d13eee71c36c` | `opencode-907b3bc518fa48e90e8ec24dd327d13eee71c36c` |

Pi 原 `badlogic/pi-mono` 地址当前跳转至 `earendil-works/pi`。Git HTTPS 获取未成功，改用 GitHub API 确认 SHA，再获取对应 codeload 归档。Codex 官方文档页返回 403，本文不引用未读取的文档内容。

## 1. Pi：保留运行历史，在模型入口整理上下文

源码：[`agent-loop.ts`](https://github.com/earendil-works/pi/blob/200387122ca450d6387f033949423114a270b96c/packages/agent/src/agent-loop.ts)、[`session-manager.ts`](https://github.com/earendil-works/pi/blob/200387122ca450d6387f033949423114a270b96c/packages/coding-agent/src/core/session-manager.ts)。

- `transformContext` 在 `convertToLlm` 前执行：运行消息与本次模型输入有明确边界。
- session 使用追加式 JSONL 树，记录 `id/parentId`；压缩记录包含 `firstKeptEntryId`，构建上下文时选取摘要、保留段和之后的消息。整理模型输入不等于销毁原始历史。
- steering 和 follow-up 分开：前者在运行边界接入，后者在准备结束时接入。当前版本在一批工具完成后处理 steering，不能宣称能在每个工具之间立刻取消旧操作。
- 输出因长度截断时，即使工具参数看起来可以解析，也不执行该消息的工具调用；生成错误结果，让模型重新提出完整调用。

测试：[`agent-loop.test.ts`](https://github.com/earendil-works/pi/blob/200387122ca450d6387f033949423114a270b96c/packages/agent/test/agent-loop.test.ts)，包括上下文转换次序、截断不执行、批次后 steering、顺序执行工具等。

对 Big Head：已有历史预算、解释与观察，无需另建一套记忆库。要把“本次模型究竟看了哪些原话、哪些派生解释”变成可检查的装配结果。旧观察可以帮助理解，但不能覆盖原始输入；测试应实际检查下一次模型请求。

## 2. Codex：执行继续条件与工具边界由运行时管理

源码：[`session/turn.rs`](https://github.com/openai/codex/blob/ab45264919aaeb8a421cc156f1a5459ef9d60b72/codex-rs/core/src/session/turn.rs)、[`tools/orchestrator.rs`](https://github.com/openai/codex/blob/ab45264919aaeb8a421cc156f1a5459ef9d60b72/codex-rs/core/src/tools/orchestrator.rs)。

- 一次采样及工具处理后，运行循环同时检查模型是否需要继续与是否存在待处理输入。
- 停止 hook 可以要求继续，并将具体反馈放回循环；这不是自动证明用户目标已经完成。
- 工具权限、sandbox 和相关重试集中在执行边界，不能仅靠模型自述“可以做”来决定。
- [`context_manager/normalize.rs`](https://github.com/openai/codex/blob/ab45264919aaeb8a421cc156f1a5459ef9d60b72/codex-rs/core/src/context_manager/normalize.rs) 为缺少返回的函数调用补 `aborted` 结果，保持模型消息协议完整。这个合成返回不是外部动作没有发生的证明。
- [`session/rollout_reconstruction_tests.rs`](https://github.com/openai/codex/blob/ab45264919aaeb8a421cc156f1a5459ef9d60b72/codex-rs/core/src/session/rollout_reconstruction_tests.rs) 覆盖恢复、回退时历史与 metadata 同步、inter-agent 消息恢复等。

对 Big Head：`Agent::reason_with_tools` 已有执行循环、预算、空承诺提醒、结果校验；不是没有循环。关键是区分“模型这一轮停了”“工具返回成功”“用户目标验证通过”，并让身份、原话与调用归属在摘要和重启后仍可追溯。

## 3. OpenCode：工具状态可追踪，模型响应可以录制重播

源码：[`session/processor.ts`](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/opencode/src/session/processor.ts)。该文件是现存 processor 路径；仓库同时有新的 Core SessionRunner，不能把两套路径混称唯一架构。

- 工具按调用 ID 更新 pending/running/completed/error 状态；结束清理时，将仍未完成的工具记为 interrupted error。
- 对近期相同工具、相同参数的重复调用触发 `doom_loop` 权限处理。它只是重复模式检测，不能保证识别所有无效循环。
- 执行结束分别返回 compact、stop、continue，压缩与错误终止不是同一个概念。
- 新 Core 的 [`session-runner-recorded.test.ts`](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/core/test/session-runner-recorded.test.ts) 通过 HTTP cassette 录制/回放模型流，并接入实际 SessionRunner。
- [`session-runner-tool-events.test.ts`](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/core/test/session-runner-tool-events.test.ts) 检查失败不能发成功事件、provider step finish 不直接等于应用 step ended。

对 Big Head：保留已有 ActionRecord 和错误预算，补齐跨阶段生命周期验证。后台观察/心跳引发新 Spark 的因果链，仍需我们针对自身架构设计；不能宣称上游的重复调用检测已经解决后台自激活。

## 当前代码的对照落点

| 位置 | 已有机制 | 本轮发现的核查重点 |
| --- | --- | --- |
| `crates/big-head-ai/src/agent.rs` | 工具循环、输出警告、预算、最终校验、空承诺补救 | 约 1063 行按 `stop_reason` 决定结束；截断调用的返回配对和恢复需整链测试。约 1151 行实际逐个 await 工具，但日志可能标 parallel，观察信息与执行方式不一致 |
| `crates/big-head-loop/src/interaction.rs` | Direct/ToolLoop 分流、实际工具证据进入最终回复 | 关键词分流误判后只靠回复说明；无工具的整理阶段能否提出可执行的继续请求，需要显式设计 |
| `crates/big-head-loop/src/runtime.rs` | 前后台分流、观察后台任务、已有隔离测试 | 从新消息接入到最终回复与后台记录，是否始终保留同一来源和关联 ID |
| `experience_observer.rs` | 校验、一次修复、失败不提交、诊断测试 | 校验成功只能证明结构/来源契约，不能证明摘要没有歪曲原话 |
| `conversation_feedback.rs` | 引用匹配、纠错证据、本地与 live 测试 | 裁判模型也可能理解错；需要固定案例及人工语义评价 |

上表是针对性源码观察，不代表这些模块的全面缺陷清单。尤其“不执行截断工具”：Big Head 当前 stop 分支通常会阻止非 tool_use 响应执行，不能误报为已经证实会执行残缺参数。

## 吸收顺序与验收

### 第一批：把失败变成可重复的工程测试

沿用现有 Agent、Executor、runtime 测试入口，在最窄的模型调用边界注入固定响应或本地假服务；运行真实循环与真实校验，不在测试里复制业务逻辑。HTTP 录制内容必须去除 key 和私人对话。

| 固定输入序列 | 自动检查的结果 |
| --- | --- |
| 模型说“交给工具层”，随后提供真实写文件调用 | 第一段文字不能算文件已写；执行后独立读取临时文件；没有真的排队就不能报交接成功 |
| 工具参数类型错，下一轮修正 | 首次无写入，具体字段错误回到模型，修正后只产生预期写入 |
| 模型输出截断，包含可解析工具参数 | 不执行残缺批次，明确记录失败/未执行状态，后续恢复仍有完整调用配对 |
| 用户中途否决旧计划 | 在规定接入边界后不再派发旧计划的后续动作；已发生副作用单独报告，不能假装撤销 |
| A2A 说话者与本地用户有不同位置 | checkpoint 恢复后来源 ID 不串，装配上下文不把本地天气当 peer 的位置 |
| 观察失败，随后有新消息 | 不提交失败观察、不丢新原话、保留诊断；下一轮不会误读旧观察为新事实 |

先让这些测试暴露实际失败，再逐项修复。日志 parallel 标注可单独小修；不能用这个表面修正代替循环验证。

### 第二批：整理一条可查的执行链

利用现有记录补齐 input -> model request -> tool call -> tool result -> reply -> derived observation 的关联。区分轮次结束和目标验证结果。业务完成条件由动态组件/流程定义提供；普通聊天不强制创建计划或验收表。

用户的新决定在执行边界生效；摘要和 Lens 作为附带来源的解释输入。涉及已开始的外部动作，标明实际结果或未知，不做虚假的整体回滚承诺。

### 第三批：同一组日常案例做真实模型评测

用“平时在 ~/Desktop/p 建项目，今天只讨论，不建；之后再授权建一个临时项目”的多轮场景，分别计分：执行正确性、身份与来源、记忆准确性、自然表达、成本。增加暂停/恢复、观察失败、重启取回三个变体。隔离世界运行，完成后清理测试对象。

确定性回放验证程序，真实 MiniMax/A2A 验证模型行为，人工阅读判断是否真的听懂。三种证据分别报告，不能再用“生成了组件/观察”代替用户体验验收。

## 理论对应与边界

追加历史与派生视图对应事件溯源/物化视图的思路；执行边界对应状态机和前后置条件；固定响应重播对应确定性集成测试；实际结果与目标的差异回到下一步，对应负反馈。这里是工程类比，不是对语义正确性或收敛性的数学证明。

负反馈要有可检查的参照物，例如原话、文件内容、外部返回；再让一个模型评价另一个模型，仍然可能共同出错。上游执行框架也不能自动保证长期理解、知识正确或多层解释可靠。
