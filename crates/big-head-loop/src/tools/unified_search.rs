//! `search` tool —— unified 多源搜索,Phase 2 引入。
//!
//! LLM 一次 call 就可以拣 `kind` 走对应 chain:
//! - `web`  → Baidu/Bing/Mojeek/DDG(CJK-aware)
//! - `wiki` → Wikipedia(按 cjk 拣 zh/en)
//! - `code` → GitHub repos
//! - `news` → Hacker News stories
//!
//! Output envelope 故意唔同单源 tools(用 `results` 唔用 `data.web` 之类),
//! 方便 LLM 一眼睇出呢个系 unified 结果。
//!
//! ```json
//! {
//!   "success": true,
//!   "query": "rust async runtime",
//!   "kind": "web",
//!   "provider_used": "mojeek",
//!   "result_count": 5,
//!   "results": [
//!     { "kind": "web", "title": "...", "url": "...", "description": "...", "position": 1 },
//!     ...
//!   ]
//! }
//! ```

use std::time::Instant;

use big_head_ai::{ToolCall, ToolCallResult};
use big_head_state::StateHook;
use serde::Deserialize;

use crate::tools::search_provider::{SourceKind, chain_for_kind, contains_cjk, run_fallback_chain};

/// LLM 传嘅 `kind` 字段(序列化时细写,解析时宽)。
///
/// 暂未有 `auto` —— `web` 已经覆盖 80% 嘅 case,
/// LLM 需要 wiki/code/news 就 explicit 写。`auto` detection 可以之后补。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Web,
    Wiki,
    Code,
    News,
}

impl SearchKind {
    fn to_source_kind(self) -> SourceKind {
        match self {
            SearchKind::Web => SourceKind::Web,
            SearchKind::Wiki => SourceKind::Wiki,
            SearchKind::Code => SourceKind::Code,
            SearchKind::News => SourceKind::News,
        }
    }
}

pub(crate) async fn dispatch_unified_search(
    state_hook: Option<&StateHook>,
    call: &ToolCall,
) -> ToolCallResult {
    let query = match call.input.get("query").and_then(|v| v.as_str()) {
        Some(q) if !q.trim().is_empty() => q.trim().to_string(),
        _ => {
            return ToolCallResult {
                tool_use_id: call.id.clone(),
                output: "Missing or empty 'query' field".into(),
                is_error: true,
            };
        }
    };

    let kind: SearchKind = match call.input.get("kind") {
        Some(v) => match serde_json::from_value::<SearchKind>(v.clone()) {
            Ok(k) => k,
            Err(e) => {
                return ToolCallResult {
                    tool_use_id: call.id.clone(),
                    output: format!(
                        "Invalid 'kind' field: {e}. Expected one of: web, wiki, code, news."
                    ),
                    is_error: true,
                };
            }
        },
        None => {
            return ToolCallResult {
                tool_use_id: call.id.clone(),
                output: "Missing 'kind' field. Expected one of: web, wiki, code, news.".into(),
                is_error: true,
            };
        }
    };

    let limit = call
        .input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).clamp(1, 30))
        .unwrap_or(8);

    let source_kind = kind.to_source_kind();
    let cjk = contains_cjk(&query);
    let chain = chain_for_kind(source_kind, cjk);

    if let Some(h) = state_hook {
        let provider = chain.first().map(|p| p.name()).unwrap_or("none");
        h.emit(big_head_state::AgentActivity::WebSearch {
            query: query.clone(),
            provider: format!("unified:{}->{}", source_kind_label(source_kind), provider),
        });
    }

    let start = Instant::now();
    let outcome = run_fallback_chain(&chain, &query, limit).await;
    let duration_ms = start.elapsed().as_millis() as u64;

    match outcome {
        Ok((provider_used, hits)) => {
            if let Some(h) = state_hook {
                h.emit(big_head_state::AgentActivity::WebSearchDone {
                    query: query.clone(),
                    provider: provider_used.into(),
                    result_count: hits.len(),
                    duration_ms,
                });
            }
            let body = serde_json::json!({
                "success": true,
                "query": query,
                "kind": source_kind_label(source_kind),
                "provider_used": provider_used,
                "result_count": hits.len(),
                "results": hits,
            });
            ToolCallResult {
                tool_use_id: call.id.clone(),
                output: serde_json::to_string_pretty(&body).unwrap_or_default(),
                is_error: false,
            }
        }
        Err(e) => {
            if let Some(h) = state_hook {
                h.emit(big_head_state::AgentActivity::WebSearchDone {
                    query: query.clone(),
                    provider: "none".into(),
                    result_count: 0,
                    duration_ms,
                });
            }
            ToolCallResult {
                tool_use_id: call.id.clone(),
                output: format!("search failed: {e}"),
                is_error: true,
            }
        }
    }
}

fn source_kind_label(k: SourceKind) -> &'static str {
    match k {
        SourceKind::Web => "web",
        SourceKind::Wiki => "wiki",
        SourceKind::Code => "code",
        SourceKind::News => "news",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kind_field() {
        let v: serde_json::Value = serde_json::json!("web");
        let k: SearchKind = serde_json::from_value(v).unwrap();
        assert!(matches!(k, SearchKind::Web));

        let v: serde_json::Value = serde_json::json!("wiki");
        let k: SearchKind = serde_json::from_value(v).unwrap();
        assert!(matches!(k, SearchKind::Wiki));

        let v: serde_json::Value = serde_json::json!("code");
        let k: SearchKind = serde_json::from_value(v).unwrap();
        assert!(matches!(k, SearchKind::Code));

        let v: serde_json::Value = serde_json::json!("news");
        let k: SearchKind = serde_json::from_value(v).unwrap();
        assert!(matches!(k, SearchKind::News));
    }

    #[test]
    fn rejects_unknown_kind() {
        let v: serde_json::Value = serde_json::json!("garbage");
        let r: Result<SearchKind, _> = serde_json::from_value(v);
        assert!(r.is_err());
    }

    #[test]
    fn detects_cjk_for_kind_routing() {
        // `contains_cjk` 嘅完整测试喺 search_provider.rs。
        // 呢度只 spot-check 一下,确保 dispatch 用紧嗰个版本。
        assert!(contains_cjk("你好"));
        assert!(contains_cjk("特斯拉"));
        assert!(!contains_cjk("rust async tokio"));
    }
}
