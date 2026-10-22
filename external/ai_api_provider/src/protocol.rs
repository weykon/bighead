/// 供应商使用的线协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiProtocol {
    /// Anthropic Messages API（x-api-key 头鉴权）
    Anthropic,
    /// OpenAI Chat Completions 格式（Bearer 鉴权）—— 覆盖大多数供应商
    OpenAiCompat,
    /// Google Gemini generateContent（查询参数鉴权）
    Gemini,
}
