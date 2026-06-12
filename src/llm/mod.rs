pub mod anthropic;
pub mod openai;
pub mod provider;

use crate::config::Config;
use anyhow::Result;

pub fn create_provider(config: &Config) -> Result<Box<dyn provider::LLMProvider>> {
    let model = config.model.as_deref().unwrap_or("openai/gpt-4o");
    let (provider_name, _model_id) = model.split_once('/').unwrap_or((model, ""));

    let provider_cfg = config.provider.get(provider_name);

    let default_base_url = match provider_name {
        "openrouter" => "https://openrouter.ai/api/v1",
        "groq" => "https://api.groq.com/openai/v1",
        "opencode" => "https://opencode.ai/zen/v1",
        _ => "",
    };

    let api_key = provider_cfg
        .and_then(|p| p.api_key.clone())
        .or_else(|| match provider_name {
            "openai" | "openrouter" => std::env::var("OPENAI_API_KEY").ok(),
            "groq" => std::env::var("GROQ_API_KEY").ok(),
            "opencode" => None,
            "anthropic" => std::env::var("ANTHROPIC_API_KEY").ok(),
            _ => None,
        });

    let base_url = provider_cfg
        .and_then(|p| p.base_url.clone())
        .or_else(|| {
            if !default_base_url.is_empty() {
                Some(default_base_url.to_string())
            } else {
                None
            }
        });

    match provider_name {
        "openai" | "openrouter" | "groq" | "opencode" => {
            let actual_key = api_key.unwrap_or_default();
            Ok(Box::new(openai::OpenAIProvider::new(actual_key, base_url)))
        }
        "anthropic" => {
            let actual_key = api_key.ok_or_else(|| anyhow::anyhow!("{} API key not configured", provider_name))?;
            Ok(Box::new(anthropic::AnthropicProvider::new(actual_key, base_url)))
        }
        _ => anyhow::bail!(
            "Unknown provider: {}. Supported: openai, anthropic, openrouter, groq, opencode",
            provider_name
        ),
    }
}

pub fn default_model(provider_name: &str) -> &str {
    match provider_name {
        "openai" => "gpt-4o",
        "anthropic" => "claude-3-5-sonnet-latest",
        "openrouter" => "openai/gpt-4o",
        "groq" => "llama-3.1-8b-instant",
        "opencode" => "big-pickle",
        _ => "gpt-4o",
    }
}
