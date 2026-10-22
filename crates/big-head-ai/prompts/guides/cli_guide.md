# 命令执行（execute_cli）

你有 `execute_cli` 工具可以跑 shell 命令。`sh -c` 执行，返回 stdout/stderr
（智能截断 + 错误行提取 + 完整日志落盘）。

## 参数
- `command` (required) — shell 命令
- `working_dir` — 工作目录
- `timeout_secs` — 超时秒数（默认 120，最大 600）
- `description` — 人类可读描述
- `background` (bool) — 后台运行（见下）
- `name` — 后台程序在 TUI Processes 中显示的稳定名称
- `purpose` — 为什么启动它，会作为 ECS 事实保存
- `port` — 可选的服务端口
- `health_url` — 可选的健康检查地址（当前先记录，健康探测随后由 supervisor 扩展）

## 何时用 `background: true`

适用于 **长时间运行的进程**：
- Dev server（`npm run dev`, `cargo run`, `vite`）
- 守护进程（`redis-server`, `postgres`）
- 任何阻塞 + 不会自然退出的命令

工作方式：
- 使用 detached wrapper 启动，脱离当前终端
- stdout/stderr 重定向到 `.big-head/outputs/bg_<pid>.log`
- 立即返回 PID、log 路径和 ManagedProcess id，并在 TUI 的 Processes 页可见

**重要限制**：
- `timeout_secs` 对 background 命令 **无效**（wrapper 立刻返回，进程自己跑到死）
- 没有 live output — 需要看进度时 `cat` log 文件
- 进程会一直跑直到自己退出；要停就 `kill <pid>`
- 可以调用 `inspect_processes` 查看 ECS 中登记的所有后台程序。runtime 每 5 秒复核
  PID；启动后返回成功不代表服务永远健康，最终状态以 Processes 页的 Running/Exited 为准。

判断要不要 background：
- 命令会跑几秒以上 + 你想继续做其他事 → `background: true`
- 命令期望拿到结果（test / build / query）→ 默认 false（同步）

## Security

- Security validator 会拦截 `rm -rf /`, `mkfs.*`, fork bomb, `curl|sh` 等灾难性命令
- Command guard 也会拦截低价值高成本的全盘探索，例如 `find / ...`, `find /Users ...`,
  `grep -R / ...`。这些命令会拖慢 wave，并且通常说明你没有利用已有 ECS/日志痕迹。
- 无论是否 background，security check 一致
- 不要尝试 bypass — 改用其他方式

## 搜索策略

不要从 `/` 或整个 `/Users` 开始找东西。优先顺序：

1. 查当前 ECS/工具痕迹：`query_entities("ActionRecord")`, `query_entities("TransactionRecord")`,
   `query_entities("ShardData")`, `query_entities("ViewState")`，或使用明确组件名查询。
2. 查运行日志：`ls -t outputs/run_*.log | head -1` 后只读最近几十行；需要更早结果时用关键词
   限定到 `outputs/`。
3. 查项目文件：`rg --files`, `rg <keyword> crates docs`, `find . -name ... -not -path './target/*'`。
4. 只有用户明确要求系统级排查时，才考虑特定绝对目录；仍要限定目录、深度和结果数量。

如果一次探索失败，要在下一步复用失败信息：记录“查过哪里、没有什么、下一步为什么换路径”。
不要反复从最高入口重新扫描。
