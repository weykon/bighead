# Bighead

Bighead 是一个 Rust ECS agent 实验项目：以持久化 world 承载事实和运行状态，由模型、工具与 Rhai 脚本共同操作，提供终端、浏览器视图及 macOS 桌面入口。

项目仍在积极开发，接口与数据格式尚未稳定。[Smallhead](https://github.com/weykon/smallhead) 是相关的终端 agent 项目，复用 ECS 基础并探索 Lens 策略与 Monad 协作。

## 快速开始

需要支持 Rust 2024 / 当前 vendored Bevy 的 Rust 工具链。开发环境可使用 Rust 1.99.0：

```sh
git clone https://github.com/weykon/bighead.git big-head
cd big-head
rustup toolchain install 1.99.0 --profile minimal
export MINIMAX_API_KEY='your-api-key'
cargo +1.99.0 run -p big-head -- --tui
```

也可将 `.env.example` 复制为本机 `.env` 后填入密钥。默认入口目前使用 MiniMax；底层多 provider 支持不等于入口已支持任意模型配置。

```sh
# 普通终端入口
cargo +1.99.0 run -p big-head

# 无交互桌面服务入口；A2A 需显式开启
cargo +1.99.0 run -p big-head -- --headless --a2a-server
```

应用会在本机生成 world、checkpoint、日志与视图快照。初次启动没有作者的运行状态或个人记忆。当前主要在 macOS 开发，macOS 原生入口需要 Swift / Xcode 工具链。

## 主要模块

| 模块 | 职责 |
| --- | --- |
| `big-head-loop` | AgentRuntime、运行调度、解释与交互层、工具、Rhai 桥接 |
| `big-head-ai` | 模型交互、system prompt 与工具协议 |
| `big-head-ecs` | ECS 数据类型、组件和注册 |
| `big-head-query` | World 查询 |
| `big-head-script` | Rhai 执行 |
| `big-head-save` | checkpoint 与恢复 |
| `big-head-shard` | 数据分片与存储治理 |
| `big-head-state` | 活动事件与状态投影 |
| `big-head-view` | 浏览器视图协议和服务 |
| `big-head-world` | World 身份、路径与应用配置 |
| `big-head-cli` | 命令执行能力 |
| `external/ai_api_provider` | API provider、请求、重试与 token 预算 |
| `external/big-head-menubar` | macOS Swift 桌面入口 |
| `vendor` | 随项目保留的 Bevy 源码与原许可证 |

浏览器前端在 `crates/big-head-view/frontend`：

```sh
cd crates/big-head-view/frontend
npm ci
npm run dev
```

这会启动前端开发服务器；后端需从仓库根目录使用 `cargo +1.99.0 run -p big-head -- --view-server` 另外启动。也可先执行 `npm run build`，再由后端提供 dist 静态文件。View 服务默认只监听 `127.0.0.1`，不提供项目根目录中的文件。

更多运行方式见 [macOS 入口](external/big-head-menubar/README.md)、[A2A](docs/a2a-local-bridge.md) 和 [开源快照说明](PUBLICATION.md)。

## 开发

```sh
cargo +1.99.0 check --workspace
cargo +1.99.0 test --workspace
```

文档中保留了不同阶段的设计和实验记录，不能将设计目标全部视为当前已实现行为。模型、Python 环境及本地服务需按相应集成文档配置，不包含在源码中。

## License

项目原创代码采用 [MIT](LICENSE) 许可证。Vendored Bevy 与其他第三方组件保留各自许可证和声明。
