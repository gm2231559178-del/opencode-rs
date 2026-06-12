use super::provider::*;
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.anthropic.com/v1".into()),
        }
    }
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    async fn generate(&self, request: &LLMRequest) -> Result<LLMResponse> {
        let body = build_messages_body(request);
        let resp = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .context("Anthropic API request failed")?;

        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("Anthropic API error {}: {}", status, text);
        }

        let data: Value = serde_json::from_str(&text)?;

        let content = data["content"]
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|c| c["type"] == "text")
                    .and_then(|c| c["text"].as_str().map(String::from))
                })
            .unwrap_or_default();

        let tool_calls = data["content"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter(|c| c["type"] == "tool_use")
                    .map(|tc| ToolCall {
                        id: tc["id"].as_str().unwrap_or_default().to_string(),
                        name: tc["name"].as_str().unwrap_or_default().to_string(),
                        arguments: tc["input"].clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let finish_reason = match data["stop_reason"].as_str() {
            Some("end_turn") => FinishReason::Stop,
            Some("tool_use") => FinishReason::ToolCalls,
            Some("max_tokens") => FinishReason::Length,
            _ => FinishReason::Stop,
        };

        let usage = data["usage"].as_object().map(|u| Usage {
            prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
            usage,
        })
    }

    async fn stream(&self, request: &LLMRequest) -> Result<BoxStream<'static, LLMEvent>> {
        let mut body = build_messages_body(request);
        body["stream"] = json!(true);

        let resp = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .context("Anthropic stream request failed")?;

        let stream = resp.bytes_stream().map(|chunk| {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk);
            let events = parse_sse_events(&text);
            Ok::<Vec<LLMEvent>, anyhow::Error>(events)
        });

        Ok(stream
            .flat_map(|result| futures::stream::iter(result.unwrap_or_default()))
            .boxed())
    }
}

fn build_messages_body(request: &LLMRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();

    for msg in &request.messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "user",
            Role::System => "user",
        };

        let content: Vec<Value> = msg
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => {
                    Some(json!({"type": "text", "text": text}))
                }
                ContentPart::ToolResult { tool_call_id, content } => {
                    Some(json!({
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": content
                    }))
                }
                ContentPart::ToolCall { id, name, arguments } => {
                    Some(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": arguments
                    }))
                }
            })
            .collect();

        messages.push(json!({"role": role, "content": content}));
    }

    let system = request.system.as_deref().unwrap_or_default();

    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema
            })
        })
        .collect();

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "system": system,
        "tools": tools,
        "max_tokens": request.max_tokens.unwrap_or(4096),
    });

    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    body
}

fn parse_sse_events(text: &str) -> Vec<LLMEvent> {
    let mut events = Vec::new();
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(val) = serde_json::from_str::<Value>(data) {
                match val["type"].as_str() {
                    Some("content_block_delta") => {
                        if val["delta"]["type"] == "text_delta" {
                            events.push(LLMEvent::Text {
                                delta: val["delta"]["text"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string(),
                            });
                        }
                    }
                    Some("content_block_start") => {
                        if val["content_block"]["type"] == "tool_use" {
                            events.push(LLMEvent::ToolCallStart {
                                id: val["content_block"]["id"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string(),
                                name: val["content_block"]["name"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string(),
                            });
                        }
                    }
                    Some("message_delta") => {
                        if let Some(stop) = val["delta"]["stop_reason"].as_str() {
                            let reason = match stop {
                                "end_turn" => FinishReason::Stop,
                                "tool_use" => FinishReason::ToolCalls,
                                "max_tokens" => FinishReason::Length,
                                _ => FinishReason::Stop,
                            };
                            let usage = val["usage"].as_object().map(|u| Usage {
                                prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
                                completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
                            });
                            events.push(LLMEvent::Finish {
                                finish_reason: reason,
                                usage,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    events
}
