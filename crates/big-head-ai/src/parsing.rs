//! [`AgentError`] —— agent 操作的错误类型。

/// Agent 操作的错误。
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("API error: {0}")]
    ApiError(String),
    #[error("Local result validation failed: {0}")]
    ResultValidation(String),
    #[error("Local context budget exceeded: {0}")]
    ContextBudget(String),
    #[error("Local context projection failed: {0}")]
    ContextProjection(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_failures_are_not_reported_as_api_errors() {
        for error in [
            AgentError::ResultValidation("reason too long".into()),
            AgentError::ContextBudget("over limit".into()),
            AgentError::ContextProjection("storage failure".into()),
        ] {
            assert!(error.to_string().starts_with("Local "));
            assert!(!error.to_string().contains("API error"));
        }
        assert_eq!(
            AgentError::ApiError("HTTP 429".into()).to_string(),
            "API error: HTTP 429"
        );
    }
}
