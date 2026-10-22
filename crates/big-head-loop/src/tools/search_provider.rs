//! `SearchProvider` trait + 统一 hit 信封 + 健康度感知嘅 fallback runner。
//!
//! 目标:取代分散喺 `web_search` / `wiki` / `brave` / `github` / `hackernews` 5 个文件
//! 各自一套 `SearchHit*` struct,统一为一个 `SearchHit` enum。
//!
//! ## 现状 → 目标
//!
//! 5 个文件 4 个唔同形状嘅 hit struct,改动要喺 4 个地方同步;改成 trait 之后:
//! - 加新 source:实现 `SearchProvider` 即可,无需改 envelope
//! - LLM 见到 discriminated union (`serde(tag = "kind")`),自己知道点用
//! - `provider_health::global()` 嘅 name 用 trait `name()` 拎,唔再 hardcode 字符串
//!
//! ## Phase 范围
//!
//! Phase 1(本 commit):trait + enum + 5 impl,`dispatch_*` 暂时保留旧逻辑
//! Phase 2:加 unified `search` tool + router,旧 5 tool 改 thin wrapper
//! Phase 3:prompt + tool_definitions 迁移,旧 tool 标 deprecated
//!
//! ## 用法
//!
//! ```ignore
//! use crate::tools::search_provider::{SearchProvider, SearchHit, run_fallback_chain};
//!
//! // 单个 source:
//! let hits: Vec<SearchHit> = MojeekProvider.search(q, 8).await?;
//!
//! // Fallback chain (CJK-aware 喺 web_search.rs 拣链):
//! let chain: Vec<&dyn SearchProvider> = vec![&Baidu, &BingCn, &Mojeek, &DuckDuckGo];
//! let hits = run_fallback_chain(&chain, q, 8).await?;
//! ```

use std::time::Instant;

use serde::Serialize;

use crate::tools::provider_health::{self, SharedHealthRegistry};

/// 命中结果统一信封。
///
/// 用 `#[serde(tag = "kind")]` LLM 收到嘅 JSON 类似:
/// ```json
/// { "kind": "repo", "full_name": "...", "stars": 95000, ... }
/// ```
/// 同 `SourceKind` 对齐嘅 4 个 variant,各自带 source-specific metadata
/// (stars / points / language 等)——砍咗就系 normalize,信息量会跌。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchHit {
    /// 通用网页结果(mojeek / ddg / bing / baidu / brave)
    Web {
        title: String,
        url: String,
        description: String,
        position: usize,
    },
    /// Wikipedia(中英文)
    Wiki {
        title: String,
        url: String,
        snippet: String,
        lang: String,
        position: usize,
    },
    /// GitHub repo
    Repo {
        full_name: String,
        html_url: String,
        description: String,
        language: String,
        stars: u64,
        forks: u64,
        position: usize,
    },
    /// HackerNews story
    Story {
        title: String,
        url: String,
        author: String,
        points: u64,
        num_comments: u64,
        created_at: String,
        position: usize,
    },
}

/// Source 嘅用途分类,俾 router 拣默认 chain。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// 通用网页(mojeek / ddg / bing / baidu / brave)
    Web,
    /// Wikipedia
    Wiki,
    /// GitHub repo
    Code,
    /// HackerNews
    News,
}

/// 统一搜索 source 嘅 interface。
///
/// 实现者 = 5 个现成 search 函数嘅新家:
/// - `MojeekProvider` / `DuckDuckGoProvider` / `BingCnProvider` / `BingComProvider` / `BaiduProvider` (喺 `web_search.rs`)
/// - `BraveProvider` (喺 `brave.rs`)
/// - `WikipediaProvider` (喺 `wiki.rs`)
/// - `GithubProvider` (喺 `github.rs`)
/// - `HackerNewsProvider` (喺 `hackernews.rs`)
#[async_trait::async_trait]
pub trait SearchProvider: Send + Sync {
    /// Stable name —— `provider_health` registry 嘅 key,fallback log 嘅标签。
    /// 必须全小写、无空格,例如 `"mojeek"` / `"bing-cn"` / `"github"`。
    fn name(&self) -> &'static str;

    /// 用作 router 默认链选择。同 kind 嘅 source 可以互相 fallback(eg Web 链)。
    fn kind(&self) -> SourceKind;

    /// 真正嘅网络请求 + 解析。
    /// 返 `Ok(vec![])` 系合法嘅(0 hits),唔好当 error。
    /// 返 `Err(String)` 会俾 `run_fallback_chain` 喂俾 `provider_health::classify_error`。
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String>;
}

/// 俾 unified `search` tool 拣 chain 嘅路由表。
///
/// `cjk` flag 决定 Web chain 用 zh 系(Baidu + cn.bing)定 en 系(bing.com),
/// 其他 kind 唔受 cjk 影响(wiki 嘅 cjk 喺调用方按 query 内容拣 lang)。
pub fn chain_for_kind(kind: SourceKind, cjk: bool) -> Vec<&'static dyn SearchProvider> {
    use crate::tools::github::GithubProvider;
    use crate::tools::hackernews::HackerNewsProvider;
    use crate::tools::web_search::{
        BaiduProvider, BingCnProvider, BingComProvider, DuckDuckGoProvider, MojeekProvider,
    };
    use crate::tools::wiki::WikipediaProvider;

    match kind {
        SourceKind::Web => {
            if cjk {
                vec![
                    &BaiduProvider,
                    &BingCnProvider,
                    &MojeekProvider,
                    &DuckDuckGoProvider,
                ]
            } else {
                vec![&BingComProvider, &MojeekProvider, &DuckDuckGoProvider]
            }
        }
        SourceKind::Wiki => {
            // 按 query cjk 拣 zh/en wiki,各自走单源(冇 fallback chain)
            if cjk {
                vec![WikipediaProvider::chinese()]
            } else {
                vec![WikipediaProvider::english()]
            }
        }
        SourceKind::Code => vec![&GithubProvider],
        SourceKind::News => vec![&HackerNewsProvider],
    }
}

/// 在 chain 中按顺序尝试每个 source,首个返 nonempty hits 就停。
///
/// 应用嘅规则(同旧 `web_search::perform_search` 一致):
/// 1. `provider_health::should_skip` 嘅 source 直接 skip,log 原因
/// 2. `Err(_)` 喂俾 `classify_error` 记入 health
/// 3. `Ok(0 hits)` 唔当 error,但系算 success(技术上 fetch 通)
/// 4. `Ok(hits)` 且非空 → 记 success,return `(name, hits)` 让 caller 知道边个 source 成功
///
/// 如果全部 skip + 全部 0 hits + 全部 error,返 `Err(last_err)` 或带 tried 列表嘅 fallback 消息。
///
/// 用 `&'static` 约束令 chain 可以直接装 `&ProviderStruct`(unit-struct value pattern)
/// 或者 `&'static WikipediaProvider` 之类嘅 statics,zero-cost。
pub async fn run_fallback_chain(
    chain: &[&'static dyn SearchProvider],
    query: &str,
    limit: usize,
) -> Result<(&'static str, Vec<SearchHit>), String> {
    let health: SharedHealthRegistry = provider_health::global();
    let start = Instant::now();
    let mut last_err: Option<String> = None;
    let mut tried: Vec<&'static str> = Vec::with_capacity(chain.len());

    for provider in chain {
        let name = provider.name();
        let kind = provider.kind();

        // Skip check
        let skip_reason: Option<String> = {
            let reg = health.lock().unwrap();
            if reg.should_skip(name) {
                let h = reg.health(name);
                Some(format!(
                    "score={:.2}, consecutive_failures={}, last_error={:?}",
                    h.score, h.consecutive_failures, h.last_error
                ))
            } else {
                None
            }
        };
        if let Some(reason) = skip_reason {
            eprintln!(
                "[search_chain] SKIP {name} ({:?}, {reason}), trying next",
                kind
            );
            tried.push(name);
            continue;
        }

        tried.push(name);
        match provider.search(query, limit).await {
            Ok(hits) if !hits.is_empty() => {
                health.lock().unwrap().record_success(name);
                eprintln!(
                    "[search_chain] {name} returned {} hits in {:?}, done",
                    hits.len(),
                    start.elapsed()
                );
                return Ok((name, hits));
            }
            Ok(_) => {
                eprintln!("[search_chain] {name} returned 0 hits, trying next");
                health.lock().unwrap().record_success(name);
            }
            Err(e) => {
                let kind = provider_health::classify_error(&e);
                eprintln!(
                    "[search_chain] {name} error ({:?}): {e}, trying next",
                    kind.label()
                );
                last_err = Some(e);
                health.lock().unwrap().record_failure(name, kind);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        format!(
            "all {} sources returned 0 hits or skipped (tried: {:?})",
            chain.len(),
            tried
        )
    }))
}

/// 探测 query 系咪含 CJK(中/日/韩)字符,俾 `chain_for_kind` 拣 zh/en variant。
///
/// 规则同 Wikipedia 嘅 lang auto-detect 一致,但范围更广(包含 Hiragana / Katakana / Hangul),
/// 用喺决定 search chain 嘅 CJK-aware 路由。
pub fn contains_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        let cp = c as u32;
        (0x4E00..=0x9FFF).contains(&cp)        // CJK Unified Ideographs
            || (0x3400..=0x4DBF).contains(&cp)  // CJK Unified Ideographs Extension A
            || (0x3040..=0x309F).contains(&cp)  // Hiragana
            || (0x30A0..=0x30FF).contains(&cp)  // Katakana
            || (0xAC00..=0xD7AF).contains(&cp) // Hangul Syllables
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::brave::BraveProvider;
    use crate::tools::github::GithubProvider;
    use crate::tools::hackernews::HackerNewsProvider;
    use crate::tools::wiki::WikipediaProvider;

    /// Lock in each provider's name + kind — these are the keys that
    /// `provider_health` and the Phase 2 router rely on.
    #[test]
    fn provider_names_and_kinds() {
        assert_eq!(BraveProvider.name(), "brave");
        assert_eq!(BraveProvider.kind(), SourceKind::Web);

        assert_eq!(GithubProvider.name(), "github");
        assert_eq!(GithubProvider.kind(), SourceKind::Code);

        assert_eq!(HackerNewsProvider.name(), "hackernews");
        assert_eq!(HackerNewsProvider.kind(), SourceKind::News);

        assert_eq!(WikipediaProvider::english().name(), "wikipedia");
        assert_eq!(WikipediaProvider::english().kind(), SourceKind::Wiki);
        assert_eq!(WikipediaProvider::chinese().name(), "wikipedia");
        assert_eq!(WikipediaProvider::chinese().kind(), SourceKind::Wiki);
    }

    #[test]
    fn contains_cjk_detects_chinese() {
        assert!(contains_cjk("你好"));
        assert!(contains_cjk("Tesla 特斯拉"));
        assert!(contains_cjk("中華人民共和國"));
    }

    #[test]
    fn contains_cjk_detects_japanese_korean() {
        assert!(contains_cjk("こんにちは")); // Hiragana
        assert!(contains_cjk("カタカナ")); // Katakana
        assert!(contains_cjk("안녕하세요")); // Hangul
    }

    #[test]
    fn contains_cjk_rejects_pure_ascii() {
        assert!(!contains_cjk("hello world"));
        assert!(!contains_cjk("rust async tokio"));
        assert!(!contains_cjk(""));
    }
}
