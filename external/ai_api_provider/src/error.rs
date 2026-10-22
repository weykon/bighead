use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("API error {status}: {body}")]
    ApiResponse { status: u16, body: String },

    #[error("Failed to parse response: {0}")]
    Parse(String),

    #[error("No API key found for {provider} (checked env vars: {env_vars})")]
    NoApiKey { provider: String, env_vars: String },

    #[error("API queue full ({depth} requests waiting, max exceeded)")]
    QueueFull { depth: usize },
}
