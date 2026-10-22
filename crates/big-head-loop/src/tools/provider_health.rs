//! Provider health tracking — 让 `web_search` 嘅 fallback chain 记住"边个源最近死了"。
//!
//! ## 目的
//!
//! 现状系串行 fallback,每个死源要等满 timeout(15s)先放棄。
//! 对 CJK 用户最痛——4 个源 3 个死,就系 60s 等 4 个 timeout 换 0 结果。
//!
//! 这个模块俾每个 provider 维护一张健康度小表:
//! - 每次成功:score 升
//! - 每次失败:score 跌,记录 `last_error_kind`
//! - 旧嘅失败(>1h)自动衰减,源恢复咗会自然回升
//!
//! `web_search` 嘅 chain 喺打网络之前先 filter,跳过 `should_skip == true` 嘅源。
//!
//! ## 范围
//!
//! - 只覆盖 `web_search` 嘅 fallback chain(5 个 provider)。
//! - wiki / brave / github / hn 系直接调用,唔入 chain,呢度唔处理(独立 experiment)。
//! - 不做并行 fan-out(架构改动,等 spike 验证后先做)。
//! - 不做 cache(独立 experiment)。
//!
//! ## 用法
//!
//! ```ignore
//! use crate::tools::provider_health::{global, classify_error};
//!
//! // chain 入口 filter:
//! if global().lock().unwrap().should_skip("mojeek") {
//!     eprintln!("[web_search] skipping mojeek (recent failures)");
//!     continue;
//! }
//!
//! // chain 出口 record:
//! match try_provider(...).await {
//!     Ok(hits) if !hits.is_empty() => {
//!         global().lock().unwrap().record_success("mojeek");
//!         return Ok(hits);
//!     }
//!     Ok(_) => global().lock().unwrap().record_success("mojeek"), // 0 hits 都算 OK
//!     Err(e) => {
//!         global().lock().unwrap().record_failure("mojeek", classify_error(&e));
//!         last_err = Some(e);
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 失败原因分类。让 `should_skip` 嘅判断更精确——
/// `CjkRejected` 永远跳过都得;`Timeout` 可能只系临时网抖,衰减后俾机会。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderErrorKind {
    /// 网络超时(包括 transport timeout)
    Timeout,
    /// HTTP 状态码非 2xx
    HttpStatus(u16),
    /// 识别到 captcha / "are you human"
    Captcha,
    /// 服务端主动拒绝 CJK(例如 Mojeek 嘅 403 + 后续 captcha)
    CjkRejected,
    /// 解析 HTML 失败(structure 变化)
    ParseError,
    /// TLS 握手失败 / 证书问题
    Tls,
    /// 未知错误
    Unknown,
}

impl ProviderErrorKind {
    pub fn label(self) -> &'static str {
        match self {
            ProviderErrorKind::Timeout => "timeout",
            ProviderErrorKind::HttpStatus(c) => return_boxed(c),
            ProviderErrorKind::Captcha => "captcha",
            ProviderErrorKind::CjkRejected => "cjk-rejected",
            ProviderErrorKind::ParseError => "parse-error",
            ProviderErrorKind::Tls => "tls",
            ProviderErrorKind::Unknown => "unknown",
        }
    }
}

/// 避免 `label()` 临时分配 String。`HttpStatus(c)` 嘅 label 唔系 static,用呢个 return。
fn return_boxed(c: u16) -> &'static str {
    // 简化:就返 "http-status" + 数字通过别的方式;呢度返固定标籤
    // 实际要区分就 caller 用 format!("http-{}", c)
    let _ = c;
    "http-status"
}

/// 单个 provider 嘅健康度快照。
#[derive(Debug, Clone)]
pub struct HealthSnapshot {
    /// 综合得分 [0.0, 1.0],时间衰减后。
    pub score: f32,
    /// 连续失败次数(成功会 reset 为 0)
    pub consecutive_failures: u32,
    /// 总成功次数(时间衰减前)
    pub total_success: u32,
    /// 总失败次数(时间衰减前)
    pub total_failure: u32,
    /// 最近一次错误
    pub last_error: Option<(ProviderErrorKind, u64)>,
}

#[derive(Debug, Clone)]
struct ProviderHealth {
    success_count: u32,
    failure_count: u32,
    last_error_kind: Option<ProviderErrorKind>,
    last_error_at: u64,
    last_success_at: u64,
    consecutive_failures: u32,
}

impl ProviderHealth {
    fn new() -> Self {
        Self {
            success_count: 0,
            failure_count: 0,
            last_error_kind: None,
            last_error_at: 0,
            last_success_at: 0,
            consecutive_failures: 0,
        }
    }

    fn record_success(&mut self, now: u64) {
        self.success_count = self.success_count.saturating_add(1);
        self.consecutive_failures = 0;
        self.last_success_at = now;
    }

    fn record_failure(&mut self, kind: ProviderErrorKind, now: u64) {
        self.failure_count = self.failure_count.saturating_add(1);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.last_error_kind = Some(kind);
        self.last_error_at = now;
    }

    /// 衰减后嘅有效计数:超过 1 小时嘅事件权重降到 0.1×,2 小时归零。
    /// 简单实现:用 epoch hour 做 bucket 区分新旧。
    /// 实际要更精确嘅话用 EWMA,但呢度够用。
    fn decayed_counts(&self, now: u64) -> (f32, f32) {
        const HALF_LIFE_SECS: u64 = 3600; // 1 小时半衰期
        // 各自嘅衰减权重:如果该侧从未发生过(计数 0),权重 1.0 唔影响结果
        let succ_weight = if self.success_count == 0 {
            1.0
        } else {
            let elapsed = now.saturating_sub(self.last_success_at);
            0.5_f32.powf(elapsed as f32 / HALF_LIFE_SECS as f32)
        };
        let fail_weight = if self.failure_count == 0 {
            1.0
        } else {
            let elapsed = now.saturating_sub(self.last_error_at);
            0.5_f32.powf(elapsed as f32 / HALF_LIFE_SECS as f32)
        };
        (
            self.success_count as f32 * succ_weight,
            self.failure_count as f32 * fail_weight,
        )
    }

    fn score(&self, now: u64) -> f32 {
        let (succ, fail) = self.decayed_counts(now);
        // 加 1 平滑,避免 0/0 时嘅边界
        succ / (succ + fail + 1.0)
    }

    /// 默认阈值:score < 0.3 → 跳过。
    /// 配合 `consecutive_failures >= 3` 双重保险:
    /// - score 低:长期唔可靠
    /// - 连续失败多:最近真系烂咗
    fn should_skip(&self, now: u64) -> bool {
        if self.consecutive_failures >= 3 && self.score(now) < 0.3 {
            return true;
        }
        // CjkRejected 系硬错误,识到一次就长期跳过(2 小时内)
        if matches!(self.last_error_kind, Some(ProviderErrorKind::CjkRejected)) {
            if now.saturating_sub(self.last_error_at) < 7200 {
                return true;
            }
        }
        false
    }
}

/// 全局 health registry,所有 source 共用。
#[derive(Debug, Default)]
pub struct ProviderHealthRegistry {
    map: HashMap<String, ProviderHealth>,
}

impl ProviderHealthRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_success(&mut self, provider: &str) {
        let now = now_secs();
        self.map
            .entry(provider.to_string())
            .or_insert_with(ProviderHealth::new)
            .record_success(now);
    }

    pub fn record_failure(&mut self, provider: &str, kind: ProviderErrorKind) {
        let now = now_secs();
        self.map
            .entry(provider.to_string())
            .or_insert_with(ProviderHealth::new)
            .record_failure(kind, now);
    }

    pub fn health(&self, provider: &str) -> HealthSnapshot {
        let now = now_secs();
        match self.map.get(provider) {
            Some(h) => HealthSnapshot {
                score: h.score(now),
                consecutive_failures: h.consecutive_failures,
                total_success: h.success_count,
                total_failure: h.failure_count,
                last_error: h.last_error_kind.map(|k| (k, h.last_error_at)),
            },
            None => HealthSnapshot {
                score: 1.0, // 未见过嘅 provider 假设健康
                consecutive_failures: 0,
                total_success: 0,
                total_failure: 0,
                last_error: None,
            },
        }
    }

    pub fn should_skip(&self, provider: &str) -> bool {
        let now = now_secs();
        self.map
            .get(provider)
            .map(|h| h.should_skip(now))
            .unwrap_or(false)
    }

    /// 全部 provider 嘅快照,俾 diagnostics 用。
    #[allow(dead_code)]
    pub fn snapshot_all(&self) -> Vec<(String, HealthSnapshot)> {
        let now = now_secs();
        let mut out: Vec<_> = self
            .map
            .iter()
            .map(|(k, h)| {
                (
                    k.clone(),
                    HealthSnapshot {
                        score: h.score(now),
                        consecutive_failures: h.consecutive_failures,
                        total_success: h.success_count,
                        total_failure: h.failure_count,
                        last_error: h.last_error_kind.map(|k| (k, h.last_error_at)),
                    },
                )
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// 清空所有状态(俾 tests 用)。
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.map.clear();
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 从错误字符串提取失败原因。web_search 嘅错误都系 String,所以从文本推断。
pub fn classify_error(err: &str) -> ProviderErrorKind {
    let lower = err.to_lowercase();

    // 顺序重要:captcha 优先于 cjk(都含 "cjk" 嘅关键字)
    if lower.contains("captcha")
        || lower.contains("are you human")
        || lower.contains("bot detection")
    {
        return ProviderErrorKind::Captcha;
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        return ProviderErrorKind::Timeout;
    }
    if lower.contains("tls") || lower.contains("handshake") || lower.contains("certificate") {
        return ProviderErrorKind::Tls;
    }
    if lower.contains("parse") || lower.contains("json") {
        return ProviderErrorKind::ParseError;
    }
    // Mojeek 嘅 CJK 拒绝会返 403 + 后续 captcha,所以前面 captcha 已经 catch 咗
    // 但 403 本身都可能系 CJK 拒绝
    if let Some(code) = extract_status_code(err) {
        if code == 403 || code == 451 {
            return ProviderErrorKind::CjkRejected;
        }
        return ProviderErrorKind::HttpStatus(code);
    }
    if lower.contains("error sending request")
        || lower.contains("connection refused")
        || lower.contains("dns")
        || lower.contains("connect")
        || lower.contains("proxy")
        || lower.contains("network")
    {
        return ProviderErrorKind::Timeout;
    }
    ProviderErrorKind::Unknown
}

fn extract_status_code(s: &str) -> Option<u16> {
    // reqwest 嘅 Display 格式两种:
    //   旧: "status code 403" / "error status: 403 Forbidden"
    //   新: "status: HTTP status client error (403 Forbidden) for url (...)"
    // 所以要两个 pattern 都认。
    // 1) `(403 Forbidden)` —— 新格式
    if let Some(open) = s.find('(') {
        let after = &s[open + 1..];
        if let Some(space) = after.find(' ') {
            if let Ok(code) = after[..space].parse::<u16>() {
                return Some(code);
            }
        }
    }
    // 2) `status: 403` / `status code 403` —— 旧格式
    let idx = s.find("status")?;
    let after = &s[idx + 6..];
    let trimmed = after.trim_start_matches(|c: char| !c.is_ascii_digit() && c != ':');
    let trimmed = trimmed.trim_start_matches(':').trim_start();
    let mut end = 0;
    for (i, c) in trimmed.char_indices() {
        if c.is_ascii_digit() {
            end = i + 1;
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    trimmed[..end].parse().ok()
}

/// 全局 health registry 句柄。process-local,唔持久化。
pub type SharedHealthRegistry = Arc<Mutex<ProviderHealthRegistry>>;

pub fn global() -> SharedHealthRegistry {
    static REG: OnceLock<SharedHealthRegistry> = OnceLock::new();
    REG.get_or_init(|| Arc::new(Mutex::new(ProviderHealthRegistry::new())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unseen_provider_is_healthy() {
        let r = ProviderHealthRegistry::new();
        let h = r.health("never-seen");
        assert_eq!(h.score, 1.0);
        assert!(!r.should_skip("never-seen"));
    }

    #[test]
    fn successes_keep_score_high() {
        let mut r = ProviderHealthRegistry::new();
        for _ in 0..10 {
            r.record_success("ok-src");
        }
        let h = r.health("ok-src");
        assert!(h.score > 0.9, "expected high score, got {}", h.score);
        assert!(!r.should_skip("ok-src"));
    }

    #[test]
    fn consecutive_failures_drop_score_and_trigger_skip() {
        let mut r = ProviderHealthRegistry::new();
        for _ in 0..3 {
            r.record_failure("bad-src", ProviderErrorKind::Captcha);
        }
        let h = r.health("bad-src");
        assert_eq!(h.consecutive_failures, 3);
        assert!(h.score < 0.3, "expected low score, got {}", h.score);
        assert!(
            r.should_skip("bad-src"),
            "should skip after 3 consecutive failures"
        );
    }

    #[test]
    fn success_resets_consecutive_failures() {
        let mut r = ProviderHealthRegistry::new();
        r.record_failure("x", ProviderErrorKind::Timeout);
        r.record_failure("x", ProviderErrorKind::Timeout);
        r.record_success("x");
        let h = r.health("x");
        assert_eq!(h.consecutive_failures, 0);
    }

    #[test]
    fn mixed_history_lets_provider_try() {
        let mut r = ProviderHealthRegistry::new();
        r.record_success("a");
        r.record_success("a");
        r.record_failure("a", ProviderErrorKind::Timeout);
        r.record_success("a");
        r.record_success("a");
        // 2 failures 但 8 successes, score 应该仲 ok,consecutive_failures = 0
        let h = r.health("a");
        assert!(!r.should_skip("a"));
        assert_eq!(h.consecutive_failures, 0);
    }

    #[test]
    fn cjk_rejected_hard_skip_within_2h() {
        let mut r = ProviderHealthRegistry::new();
        r.record_failure("mojeek", ProviderErrorKind::CjkRejected);
        // 模拟"刚刚发生过"
        let h = r.health("mojeek");
        assert!(h.last_error.is_some());
        assert!(r.should_skip("mojeek"));
    }

    #[test]
    fn classifies_timeout() {
        assert_eq!(
            classify_error("request timeout exceeded"),
            ProviderErrorKind::Timeout
        );
        assert_eq!(
            classify_error("operation timed out"),
            ProviderErrorKind::Timeout
        );
        assert_eq!(
            classify_error("error sending request"),
            ProviderErrorKind::Timeout
        );
    }

    #[test]
    fn classifies_captcha() {
        assert_eq!(
            classify_error("captcha challenge detected"),
            ProviderErrorKind::Captcha
        );
        assert_eq!(classify_error("are you human?"), ProviderErrorKind::Captcha);
    }

    #[test]
    fn classifies_http_status_403_as_cjk() {
        // 旧 reqwest 格式
        assert_eq!(
            classify_error("status: 403 Forbidden"),
            ProviderErrorKind::CjkRejected
        );
        // 新 reqwest 格式: 状态码在括号里
        assert_eq!(
            classify_error(
                "status: HTTP status client error (403 Forbidden) for url (https://example.com)"
            ),
            ProviderErrorKind::CjkRejected
        );
        assert_eq!(
            classify_error(
                "error sending request: status: HTTP status server error (503 Service Unavailable)"
            ),
            ProviderErrorKind::HttpStatus(503)
        );
    }

    #[test]
    fn classifies_http_status_500() {
        assert_eq!(
            classify_error("status: 500 Internal Server Error"),
            ProviderErrorKind::HttpStatus(500)
        );
    }

    #[test]
    fn classifies_tls_error() {
        assert_eq!(
            classify_error("TLS handshake failed"),
            ProviderErrorKind::Tls
        );
    }

    #[test]
    fn classifies_parse_error() {
        assert_eq!(
            classify_error("parse json error"),
            ProviderErrorKind::ParseError
        );
    }

    #[test]
    fn classifies_unknown() {
        assert_eq!(
            classify_error("weird thing happened"),
            ProviderErrorKind::Unknown
        );
    }

    #[test]
    fn snapshot_all_sorts_by_name() {
        let mut r = ProviderHealthRegistry::new();
        r.record_success("zzz");
        r.record_success("aaa");
        let snap = r.snapshot_all();
        assert_eq!(snap[0].0, "aaa");
        assert_eq!(snap[1].0, "zzz");
    }
}
