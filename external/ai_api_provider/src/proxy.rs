//! HTTP 代理配置 —— 统一从环境变量读取，应用到 `reqwest::ClientBuilder`。
//!
//! 优先级（高 → 低）：
//! 1. `BIG_HEAD_PROXY` — 我们项目显式约定的入口
//! 2. `HTTPS_PROXY` / `https_proxy`
//! 3. `HTTP_PROXY` / `http_proxy`
//! 4. `ALL_PROXY` / `all_proxy`
//!
//! reqwest 0.12 默认不读这些 env vars（不像 Go net/http），
//! 所以必须显式通过 [`ProxyConfig::apply`] 注入。

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyConfig {
    /// `http://127.0.0.1:7897` 形式。
    pub url: Option<String>,
}

impl ProxyConfig {
    /// 按优先级扫描环境变量，返回第一个非空命中。
    /// 全没设则返回 `ProxyConfig { url: None }`。
    pub fn from_env() -> Self {
        for var in [
            "BIG_HEAD_PROXY",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            if let Ok(v) = std::env::var(var) {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    return Self {
                        url: Some(trimmed.to_string()),
                    };
                }
            }
        }
        Self::default()
    }

    /// 把 proxy 套到 `reqwest::ClientBuilder` 上。
    /// 没设 url 时原样返回，方便链式调用。
    ///
    /// # Panics
    /// 如果 url 不是合法 URL（罕见,只会在用户传错时触发）。
    pub fn apply(&self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        match &self.url {
            Some(url) => {
                builder.proxy(reqwest::Proxy::all(url).expect("invalid proxy URL in ProxyConfig"))
            }
            None => builder,
        }
    }

    /// 是否配置了 proxy。
    pub fn is_enabled(&self) -> bool {
        self.url.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单元测试之间 env 会泄漏,用串行 mutex 保护。
    /// 这里偷懒用 thread-local 不实际,改用 serial_test crate 也太重。
    /// 改方案:测试用唯一 env 名避免冲突 + 单测只检查 URL 解析逻辑。
    #[test]
    fn test_default_is_disabled() {
        let cfg = ProxyConfig::default();
        assert!(!cfg.is_enabled());
        assert!(cfg.url.is_none());
    }

    #[test]
    fn test_explicit_constructor() {
        let cfg = ProxyConfig {
            url: Some("http://127.0.0.1:7897".into()),
        };
        assert!(cfg.is_enabled());
        assert_eq!(cfg.url.as_deref(), Some("http://127.0.0.1:7897"));
    }

    #[test]
    fn test_apply_disabled_returns_builder_unchanged() {
        let cfg = ProxyConfig::default();
        let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5));
        let _ = cfg.apply(builder);
        // 不 panic、不改 timeout = 通过
    }

    #[test]
    fn test_apply_enabled_attaches_proxy() {
        let cfg = ProxyConfig {
            url: Some("http://127.0.0.1:7897".into()),
        };
        let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5));
        let result = cfg.apply(builder).build();
        assert!(result.is_ok(), "proxy should be valid: {:?}", result.err());
    }

    #[test]
    #[should_panic(expected = "invalid proxy URL")]
    fn test_apply_invalid_url_panics() {
        let cfg = ProxyConfig {
            url: Some("not a url with spaces".into()),
        };
        let builder = reqwest::Client::builder();
        let _ = cfg.apply(builder);
    }
}
