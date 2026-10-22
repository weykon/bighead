//! 共享 HTTP client 构造器 —— 避免 5 个搜索工具各 copy 一次 proxy+UA+timeout 配置。

use std::time::Duration;

use big_head_ai::ProxyConfig;

/// 默认 request timeout。所有 search 工具用同一个值(15s),
/// 需要更长嘅可以喺 `build_client` 之后再 .timeout(...) 覆盖。
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 15;

/// 构造一个带 proxy + UA + timeout 嘅 `reqwest::Client`。
///
/// 集中处理嘅嘢:
/// - `ProxyConfig::from_env()` —— `BIG_HEAD_PROXY` / `HTTPS_PROXY` / `HTTP_PROXY` 等
/// - User-Agent(每个工具唔同,所以做参数)
/// - Timeout(默认 15s,大多数 search source 合理)
///
/// 返 `Result<Client, String>` 方便直接 `?`,省返 caller 写 `.map_err`。
pub(crate) fn build_client(user_agent: &str) -> Result<reqwest::Client, String> {
    ProxyConfig::from_env()
        .apply(reqwest::Client::builder())
        .user_agent(user_agent)
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("build client: {e}"))
}
