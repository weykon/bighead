# ai_api_provider

[![Crates.io Total Downloads](https://img.shields.io/crates/d/ai_api_provider)](https://crates.io/crates/ai_api_provider)
[![Crates.io Version](https://img.shields.io/crates/v/ai_api_provider)](https://crates.io/crates/ai_api_provider)
[![License: MIT](https://img.shields.io/crates/l/ai_api_provider)](LICENSE)

Multi-provider LLM API client for Rust. One unified interface for 22 providers across 3 wire protocols.

## Supported Providers

| Category | Providers |
|----------|-----------|
| **Primary** | Claude, OpenAI, Gemini |
| **Extended** | DeepSeek, Groq, Mistral, xAI (Grok), Together AI, Fireworks, Perplexity, Cohere, OpenRouter |
| **China Region** | Moonshot (Kimi), GLM (智谱), Qwen (通义), Doubao (豆包), MiniMax, Hunyuan (混元) |
| **Local** | Ollama, LM Studio, llama.cpp, vLLM |

## Usage

```rust
use ai_api_provider::{ApiClient, ApiConfig, ApiProvider, ChatMessage};

#[tokio::main]
async fn main() {
    let config = ApiConfig::new(ApiProvider::Claude, "your-api-key".into());
    let client = ApiClient::new();

    let messages = vec![
        ChatMessage::system("You are helpful."),
        ChatMessage::user("Hello!"),
    ];

    let response = client.chat(&config, &messages).await.unwrap();
    println!("{}", response);
}
```

### Auto-resolve API key from environment

```rust
use ai_api_provider::{ApiProvider, resolve_api_key};

if let Some(key) = resolve_api_key(ApiProvider::Claude) {
    // Found ANTHROPIC_API_KEY in environment
}
```

### Custom base URL (relay/proxy)

```rust
let mut config = ApiConfig::new(ApiProvider::Claude, key);
config.base_url = Some("http://my-relay:8021".into());
```

### Provider lookup by name

```rust
use ai_api_provider::provider_by_name;

let meta = provider_by_name("deepseek").unwrap();
// Also supports aliases: "grok" → xAI, "kimi" → Moonshot, etc.
```

## Wire Protocols

The crate handles 3 distinct API protocols transparently:

- **Anthropic** — `x-api-key` header, Messages API format
- **OpenAI-compatible** — `Bearer` auth, Chat Completions format (covers most providers)
- **Gemini** — query param auth, `generateContent` format

## License

MIT
