# 本机 A2A Bridge

Big Head 现在提供一个可选的本机 Agent-to-Agent HTTP bridge。它把外部
agent 的消息送入正常 Spark/coordinator 循环，不直接调用工具，也不绕过
ECS 权限和审计链路。

## 启动

```bash
cargo run -- --headless --a2a-server
```

默认只监听 `127.0.0.1:3040`。可以用 `A2A_PORT` 覆盖端口：

```bash
A2A_PORT=3041 cargo run -- --headless --a2a-server
```

这是独立于 3030 View server 的端口。A2A bridge 默认关闭，避免普通启动
时意外暴露消息入口。

## 查看 Agent Card

```bash
curl http://127.0.0.1:3040/.well-known/agent-card.json
```

也可以用仓库内的小客户端，避免手写 JSON：

```bash
scripts/a2a.sh card
scripts/a2a.sh health
scripts/a2a.sh send "请检查当前 Coding Agent 的必要条件和缺口" research
scripts/a2a.sh task task_<id>
```

客户端只负责 HTTP 编码和任务查询。它不能直接选择 ECS 工具，也不能授予
`can_mutate` 或 `can_execute` 权限；这些字段在 bridge 入口会被拒绝。

第一版只声明并允许 `observe`、`research`、`plan`、`reply`。消息请求
`can_mutate`、`can_execute` 或 `can_forward` 会被拒绝。

## 发送一条消息

```bash
curl -sS -X POST http://127.0.0.1:3040/a2a/messages \
  -H 'content-type: application/json' \
  -d '{
    "from": "codex",
    "to": "local:default",
    "kind": "request",
    "intent": "research",
    "text": "请检查当前 Coding Agent 的能力和缺口",
    "permissions": {
      "can_observe": true,
      "can_plan": true,
      "can_mutate": false,
      "can_execute": false,
      "can_forward": false
    }
  }'
```

如果 Big Head 在 120 秒内完成本轮，会直接返回 `status=done` 和真实的
Spark 摘要；如果仍在处理，则返回 `status=queued`、`task_id`，随后查询：

```bash
curl http://127.0.0.1:3040/a2a/tasks/<task_id>
```

## 边界

- HTTP 请求只进入 `InboundA2a` 通道，主循环是唯一的 Spark 创建者。
- 消息正文会标记为不可信输入；消息不能指定工具调用。
- 每条入站 Spark 附带可持久化的 `A2aOrigin`，保存 claimed `from`、`conversation_id`、`message_id` 和权限。来源未经鉴权，不等同于真实用户身份。近期对话只取同一来源声明和会话，不混入 TUI 用户对话。
- 跨新会话时会带入该 claimed peer 最早一条已完成消息的有界原文，作为“曾收到的消息”，不自动升格为可信身份、用户偏好或 FactRecord。
- `can_observe` 仅开放显式只读 ECS 工具；`execute_cli`、网络工具与所有写工具既不展示给 A2A 模型，也在执行前由工具能力检查拒绝。`can_observe=false` 不开放工具。
- `inspect_transaction(tx_id)` 只返回该 tx 的动作，并标明其 Spark 是交互用户、后台还是未经鉴权的 A2A 来源，避免把全局相邻动作错归给某个 peer。
- `A2aOrigin` 保存的是消息来源，不自动把 peer 的自述升级成 `FactRecord` 或可信长期记忆。
- 目标只能是当前 world 或 `*`，默认绑定 loopback。
- 正文最多 16 KiB，请求体最多 64 KiB，引用最多 32 个，TTL 最多 8。
- 回复状态来自 ECS `Spark` 的真实状态，不由 HTTP 层伪造完成。
- A2A reply 不会自动写入业务 ECS；是否接受请求仍由 coordinator 决定。

当前版本是本机联调桥，不是公网 A2A 服务。`from` 可由本机调用者填写，不能作为鉴权依据。远程 peer、鉴权、公钥轮换、
SSE 事件流和工具权限委托需要在本机链路验证后再增加。
