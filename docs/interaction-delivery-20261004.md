# 对话执行、用户选择与文件阅读

2026-10-04。此次实现四个通用能力，没有把项目、天气等业务类型固化进 Rust，也没有改写日常世界或重启用户进程。

## 实际使用

沿用 `cargo run -- --view-server --tui --a2a-server` 启动。View 仍按原配置使用端口，默认 3030。

- 已授权的工作直接推进；只有缺少重要资料或真正的取舍才提问。这个行为方向由提示与执行流程共同支持，不能保证每个模型回答都完美遵守。
- `request_user_input` 创建 Questions 中的问题。Ctrl+?（通常为 Ctrl+Shift+/）打开，方向键选择，Enter 确认；输入栏可填写自己的答案，Ctrl+D 取消。新问题不会打断已有输入草稿。一次展示最早的待答问题。TUI 会请求启用增强键盘协议；旧终端若将 Ctrl+? 编码为退格，无法区分两者，会保留退格行为。Ctrl+Q 不再绑定 Questions。
- 答案按问题 ID 去重，带原始请求和原交易 ID 进入正常用户输入路径。取消不触发继续操作；待答问题保存在 ECS，可随 checkpoint 恢复。
- `render_document` 在 View 创建可复用的文件页；稳定 id 更新同一页面。聊天中保留重点与返回的 `/#panel=document-...` 入口。短对话不强制生成文件。
- Activity 展示工具尝试的开始/结束、耗时与关联 ID。Agent 可用 `inspect_transaction` 查看对应 `tool_executions`，无需根据时间接近猜测归属。

自然用法示例：

> 帮我整理一个读书笔记项目的第一版方案，长内容放到 View。常规细节你决定；只有会明显影响结果而又缺资料的地方，才给我选项。

## 执行链

```text
当前消息
  -> 轻量对话，或工具执行
  -> 若轻量层发现确需工具：返回显式 request_execution，立即转执行
  -> 工具尝试：running -> succeeded / failed
  -> 最后整理：自然回复，或返回具体 continue_with
  -> 同一个执行器补做（原权限、已有证据、最多两次返回）
  -> 完成 / 未完成 / 等待用户
```

轻量对话普通回复仍用自然语言；只有需要转执行时使用严格控制结果。最后整理阶段无工具，返回结构化补做请求，由 runtime 负责真正再次调用执行器。不会递归产生 Spark，也不会自动扩大 A2A 权限。

每次补做最多 8 个 wave，并受原执行总 wave 余额约束；最多返回两次。补做失败或预算用尽标记 `Incomplete`。取消与待答时不补做。结构化整理结果校验失败会记录诊断，保留原执行报告。

## 工具与问题记录

- `ToolExecution` 是执行基础设施记录：sequence、trace_id、tx_id、tool_call_id、工具名、wave、状态、时间、耗时、有界输入/输出片段。成功仅表示该工具调用成功，不证明用户目标已经实现。
- 开始和结束均追加至已有 action log，同时维护 ECS 记录。日志写入失败沿用现有审计错误提示；没有宣称磁盘故障下仍保证落盘。
- checkpoint 中仍为 running 的记录，恢复时标记 `interrupted_unknown`，不得推断为没有副作用。没有出现在 checkpoint 中的尝试须查 action log；本版不自动从日志重放外部动作。
- `UserQuestion` 是交互基础设施记录：问题、选项、原请求、交易关联、回答与状态。工具只在启用了 TUI 的 runtime 中提供；执行还检查原交易是否来自本地交互消息，后台和 A2A 不能借此冒充本地用户提问。
- 问题建立后当前工具批次停止，不执行剩下的调用。交易状态为 awaiting_input；用户回应后为 input_received，取消则为 cancelled。回答不是整个业务任务已经完成。
- 通用 ECS 写工具拒绝直接伪造/修改这两种记录。该检查不是对任意本机代码执行的隔离沙箱。

## 阅读界面

文件工具接受标题、摘要和分节文字/列表，使用 HTML 转义与现有消毒渲染，布局由前端负责。正文 16px、限制阅读宽度、段落留白，桌面和手机都可滚动阅读；文件页隐藏无关组件 ID，阅读时外层标题使用 Big Head。

View 通道关闭时返回工具错误。文件被 View 接收但 ECS 持久化失败时也明确返回错误，保留同一个 id 供修复。发布不代表用户看过，工具返回 `user_read_verified:false`。

## 验证

固定响应来自本地 HTTP 服务器，经过真实 provider adapter、Agent 循环、最终整理与 RuntimeToolExecutor，没有调用付费模型，也没有让假工具代替实际 ECS 写入。

| 测试 | 结果 |
| --- | --- |
| 漏做 -> 整理层退回 -> 参数错误 -> 修正 -> 独立读取 | 实际 ECS 值变为 7；失败与成功尝试均可查 |
| 已建立问题后同批次还有写入调用 | 后续调用不执行；返回 AwaitingInput |
| checkpoint 恢复问题，自填答案，重复提交 | 问题恢复；答案关联原请求；重复提交被拒 |
| 权限拒绝、恢复未结束工具尝试 | 拒绝有记录；未结束尝试标结果未知 |
| 整理层要求写入，只读执行器 | 原权限保留；写入仍被拒 |
| 截断响应没有 usage 字段 | 不执行工具，正确标记 OutputLimit；此次修复了原先误标 Completed 的情况 |
| 连续要求补做 | 两次返回后停止，标 Incomplete，无递归 Spark |
| 后台提问、通用写工具伪造问题 | 被拒绝 |
| 文件发布 | View 收到命令，ECS 有 ViewState，返回可打开路径 |
| TUI 选择、自填、取消、重复 Enter | 使用关联问题的回答事件，不当成无关 Spark |

`cargo test -p big-head-loop -p big-head-ai -p big-head-state -p big-head-view --lib`：449 passed、17 ignored。`cargo test -p big-head --bin big-head`：45 passed。前端 TypeScript/Vite 构建和主程序构建成功。ignored 没有算入通过数。

隔离 `document_demo` 在 3049 渲染，与日常 3030 世界无关。Playwright 检查 1440×1000、390×844，正文无水平溢出、无 pageerror。截图和尺寸结果在 `outputs/interaction-20261004/`。临时服务验证后关闭。

本轮没有使用真实 MiniMax 重跑自然对话，也未声称已解决所有过度询问、语义理解或观察准确性问题。已验证的是执行、权限、记录、恢复和交互边界；真实模型的选择与表达需要后续实际使用评价。
