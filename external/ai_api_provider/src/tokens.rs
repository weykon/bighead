//! LLM API 调用的 token 估算与预算追踪。
//!
//! 提供：
//! - [`model_limits`] — 按模型名称前缀查表，获取输入/输出 token 上限
//! - [`estimate_tokens`] — 快速启发式估算（零外部依赖）
//! - [`TokenBudget`] / [`BudgetStatus`] — 运行时预算追踪

/// 单个模型的 token 上限。
#[derive(Debug, Clone)]
pub struct ModelLimits {
    pub max_input_tokens: usize,
    pub max_output_tokens: usize,
}

/// 无匹配条目时的默认上限。
pub const DEFAULT_LIMITS: ModelLimits = ModelLimits {
    max_input_tokens: 128_000,
    max_output_tokens: 4_096,
};

/// 静态模型上限表，按最长前缀优先排列。
///
/// 每条为 `(前缀, ModelLimits)`，首个匹配即生效。
static MODEL_LIMITS: &[(&str, ModelLimits)] = &[
    // ── Anthropic Claude ────────────────────────────────
    (
        "claude-opus",
        ModelLimits {
            max_input_tokens: 200_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "claude-sonnet",
        ModelLimits {
            max_input_tokens: 200_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "claude-haiku",
        ModelLimits {
            max_input_tokens: 200_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "claude",
        ModelLimits {
            max_input_tokens: 200_000,
            max_output_tokens: 8_192,
        },
    ),
    // ── OpenAI ──────────────────────────────────────────
    (
        "gpt-4o",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 16_384,
        },
    ),
    (
        "gpt-4-turbo",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 4_096,
        },
    ),
    (
        "gpt-4",
        ModelLimits {
            max_input_tokens: 8_192,
            max_output_tokens: 4_096,
        },
    ),
    (
        "gpt-3.5",
        ModelLimits {
            max_input_tokens: 16_384,
            max_output_tokens: 4_096,
        },
    ),
    (
        "o1",
        ModelLimits {
            max_input_tokens: 200_000,
            max_output_tokens: 100_000,
        },
    ),
    (
        "o3",
        ModelLimits {
            max_input_tokens: 200_000,
            max_output_tokens: 100_000,
        },
    ),
    // ── Gemini ──────────────────────────────────────────
    (
        "gemini-2.0",
        ModelLimits {
            max_input_tokens: 1_048_576,
            max_output_tokens: 8_192,
        },
    ),
    (
        "gemini-1.5",
        ModelLimits {
            max_input_tokens: 1_048_576,
            max_output_tokens: 8_192,
        },
    ),
    (
        "gemini",
        ModelLimits {
            max_input_tokens: 32_768,
            max_output_tokens: 8_192,
        },
    ),
    // ── DeepSeek ────────────────────────────────────────
    (
        "deepseek-v4",
        ModelLimits {
            max_input_tokens: 1_000_000,
            max_output_tokens: 384_000,
        },
    ),
    (
        "deepseek-v3",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "deepseek-r1",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "deepseek-chat",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "deepseek",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 8_192,
        },
    ),
    // ── MiniMax（Anthropic 协议）────────────────────────
    (
        "minimax-m2",
        ModelLimits {
            max_input_tokens: 204_800,
            max_output_tokens: 4_096,
        },
    ),
    (
        "minimax",
        ModelLimits {
            max_input_tokens: 204_800,
            max_output_tokens: 4_096,
        },
    ),
    // ── MiMo / 小米（Anthropic 协议）────────────────────
    (
        "mimo-v2",
        ModelLimits {
            max_input_tokens: 256_214,
            max_output_tokens: 4_096,
        },
    ),
    (
        "mimo",
        ModelLimits {
            max_input_tokens: 256_214,
            max_output_tokens: 4_096,
        },
    ),
    // ── Mistral ─────────────────────────────────────────
    (
        "mistral-large",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 4_096,
        },
    ),
    (
        "mistral",
        ModelLimits {
            max_input_tokens: 32_000,
            max_output_tokens: 4_096,
        },
    ),
    // ── Groq（Llama 系列）───────────────────────────────
    (
        "llama-3",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "llama",
        ModelLimits {
            max_input_tokens: 8_192,
            max_output_tokens: 4_096,
        },
    ),
    // ── xAI Grok ────────────────────────────────────────
    (
        "grok",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 4_096,
        },
    ),
    // ── Moonshot / Kimi ─────────────────────────────────
    (
        "moonshot",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 4_096,
        },
    ),
    // ── GLM / 智谱 ─────────────────────────────────────
    (
        "glm",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 4_096,
        },
    ),
    // ── Qwen / 通义 ───────────────────────────────────
    (
        "qwen",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 4_096,
        },
    ),
    // ── Doubao / 字节跳动 ──────────────────────────────
    (
        "doubao",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 4_096,
        },
    ),
    // ── Cohere ──────────────────────────────────────────
    (
        "command",
        ModelLimits {
            max_input_tokens: 128_000,
            max_output_tokens: 4_096,
        },
    ),
    // ── Perplexity ──────────────────────────────────────
    (
        "sonar",
        ModelLimits {
            max_input_tokens: 127_072,
            max_output_tokens: 4_096,
        },
    ),
];

/// 根据模型名称查询 token 上限。
///
/// 按最长模型名前缀匹配。无匹配时返回 [`DEFAULT_LIMITS`]。
pub fn model_limits(model: &str) -> &'static ModelLimits {
    let lower = model.to_lowercase();
    for (prefix, limits) in MODEL_LIMITS {
        if lower.starts_with(prefix) {
            return limits;
        }
    }
    &DEFAULT_LIMITS
}

/// 快速 token 估算，无需外部依赖。
///
/// 启发式：ASCII 字符 ÷ 4，CJK 字符 × 2/3（向上取整）。
/// 对大多数 LLM 分词器误差在 ~10% 以内。
pub fn estimate_tokens(text: &str) -> usize {
    let mut count = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            // 累积计数，最后再除，避免逐字符浮点运算
            count += 1;
        } else if is_cjk(ch) {
            // CJK ≈ 每个字 2–3 token，按 2 估算
            count += 2;
        } else {
            // 其他 Unicode（emoji、带重音字符等）≈ 1.5 token
            count += 2;
        }
    }
    // ASCII 字符 ≈ 每 4 个字符 1 token；CJK 已经按 2 计算
    // 用加权计数 / 3 作为基准，至少为 1
    (count + 2) / 3
}

/// 判断字符是否在 CJK 统一表意文字区间内。
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x4E00..=0x9FFF   |  // CJK 统一表意文字
        0x3400..=0x4DBF   |  // 扩展 A
        0x20000..=0x2A6DF |  // 扩展 B
        0xF900..=0xFAFF   |  // CJK 兼容表意文字
        0x2F800..=0x2FA1F    // CJK 兼容表意文字补充
    )
}

/// 输入 token 预算状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetStatus {
    /// 预算充足。
    Ok,
    /// 接近上限（已用 ≥80%）。
    Warning { used: usize, limit: usize },
    /// 预算超限——应截断或拒绝。
    Exceeded { used: usize, limit: usize },
}

/// 追踪累计输入 token 用量，对比模型上限。
#[derive(Debug, Clone)]
pub struct TokenBudget {
    max_input: usize,
    current: usize,
}

impl TokenBudget {
    /// 创建新预算，设定最大输入 token 数。
    pub fn new(max_input: usize) -> Self {
        Self {
            max_input,
            current: 0,
        }
    }

    /// 根据模型名称创建预算（内部调用 [`model_limits`]）。
    pub fn for_model(model: &str) -> Self {
        Self::new(model_limits(model).max_input_tokens)
    }

    /// 重置预算归零。
    pub fn reset(&mut self) {
        self.current = 0;
    }

    /// 添加一段文本的估算 token 数，返回更新后的状态。
    pub fn add_text(&mut self, text: &str) -> BudgetStatus {
        self.current += estimate_tokens(text);
        self.status()
    }

    /// 添加已知的 token 数量，返回更新后的状态。
    pub fn add_tokens(&mut self, tokens: usize) -> BudgetStatus {
        self.current += tokens;
        self.status()
    }

    /// 当前预算状态。
    pub fn status(&self) -> BudgetStatus {
        if self.current > self.max_input {
            BudgetStatus::Exceeded {
                used: self.current,
                limit: self.max_input,
            }
        } else if self.current * 5 >= self.max_input * 4 {
            BudgetStatus::Warning {
                used: self.current,
                limit: self.max_input,
            }
        } else {
            BudgetStatus::Ok
        }
    }

    /// 已使用的 token 数。
    pub fn used(&self) -> usize {
        self.current
    }

    /// 最大允许输入 token 数。
    pub fn limit(&self) -> usize {
        self.max_input
    }

    /// 剩余 token 预算（下限为 0）。
    pub fn remaining(&self) -> usize {
        self.max_input.saturating_sub(self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_limits_claude() {
        let lim = model_limits("claude-sonnet-4-20250514");
        assert_eq!(lim.max_input_tokens, 200_000);
        assert_eq!(lim.max_output_tokens, 8_192);
    }

    #[test]
    fn test_model_limits_minimax() {
        let lim = model_limits("MiniMax-M2.7");
        assert_eq!(lim.max_input_tokens, 204_800);
    }

    #[test]
    fn test_model_limits_mimo() {
        let lim = model_limits("mimo-v2-pro");
        assert_eq!(lim.max_input_tokens, 256_214);
    }

    #[test]
    fn test_model_limits_deepseek_v4() {
        let lim = model_limits("deepseek-v4");
        assert_eq!(lim.max_input_tokens, 1_000_000);
    }

    #[test]
    fn test_model_limits_deepseek_chat() {
        let lim = model_limits("deepseek-chat");
        assert_eq!(lim.max_input_tokens, 128_000);
    }

    #[test]
    fn test_model_limits_unknown() {
        let lim = model_limits("some-unknown-model");
        assert_eq!(lim.max_input_tokens, 128_000);
        assert_eq!(lim.max_output_tokens, 4_096);
    }

    #[test]
    fn test_estimate_tokens_ascii() {
        // 12 个 ASCII 字符 → 约 4 token
        assert_eq!(estimate_tokens("Hello World!"), 4);
    }

    #[test]
    fn test_estimate_tokens_cjk() {
        // 4 个 CJK 字符 → 8 加权字符 → ~3 token
        let tokens = estimate_tokens("你好世界");
        assert!(tokens >= 2 && tokens <= 4);
    }

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_budget_status_ok() {
        let mut budget = TokenBudget::new(1000);
        assert_eq!(budget.status(), BudgetStatus::Ok);
        budget.add_tokens(500);
        assert_eq!(budget.status(), BudgetStatus::Ok);
    }

    #[test]
    fn test_budget_status_warning() {
        let mut budget = TokenBudget::new(1000);
        budget.add_tokens(800); // 80%
        assert!(matches!(budget.status(), BudgetStatus::Warning { .. }));
    }

    #[test]
    fn test_budget_status_exceeded() {
        let mut budget = TokenBudget::new(1000);
        budget.add_tokens(1001);
        assert!(matches!(budget.status(), BudgetStatus::Exceeded { .. }));
    }

    #[test]
    fn test_budget_for_model() {
        let budget = TokenBudget::for_model("claude-sonnet-4-20250514");
        assert_eq!(budget.limit(), 200_000);
    }

    #[test]
    fn test_budget_remaining() {
        let mut budget = TokenBudget::new(1000);
        budget.add_tokens(300);
        assert_eq!(budget.remaining(), 700);
    }
}
