use super::provider::*;
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".into()),
        }
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn generate(&self, request: &LLMRequest) -> Result<LLMResponse> {
        let body = build_chat_body(request);
        tracing::debug!(
            model = %body["model"],
            msg_count = %body["messages"].as_array().map(|a| a.len()).unwrap_or(0),
            request_body = %serde_json::to_string(&body).unwrap_or_default(),
            "generate: sending request"
        );
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .context("OpenAI API request failed")?;

        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            tracing::error!(status = %status, body = %text, "generate: API error");
            anyhow::bail!("OpenAI API error {}: {}", status, text);
        }

        let data: Value = serde_json::from_str(&text)?;
        let choice = &data["choices"][0];
        let message = &choice["message"];

        let content = message["content"].as_str().unwrap_or_default().to_string();
        let finish_reason = match choice["finish_reason"].as_str() {
            Some("stop") => FinishReason::Stop,
            Some("tool_calls") => FinishReason::ToolCalls,
            Some("length") => FinishReason::Length,
            _ => FinishReason::Stop,
        };

        let tool_calls = message["tool_calls"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|tc| ToolCall {
                        id: tc["id"].as_str().unwrap_or_default().to_string(),
                        name: tc["function"]["name"].as_str().unwrap_or_default().to_string(),
                        arguments: tc["function"]["arguments"]
                            .as_str()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or(json!({})),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let usage = data["usage"].as_object().map(|u| Usage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
            usage,
        })
    }

async fn stream(&self, request: &LLMRequest) -> Result<BoxStream<'static, LLMEvent>> {
        let mut body = build_chat_body(request);
        body["stream"] = json!(true);

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .context("OpenAI stream request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            anyhow::bail!("API error {}: {}", status, body_text);
        }

        let mut tool_ids: std::collections::HashMap<u64, String> = std::collections::HashMap::new();
        let mut line_buf = String::new();
        let stream = resp.bytes_stream().map(move |chunk| {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk);
            line_buf.push_str(&text);
            let events = parse_sse_events_from_buf(&mut line_buf, &mut tool_ids);
            Ok::<Vec<LLMEvent>, anyhow::Error>(events)
        });

        let flatten = stream.flat_map(|result| {
            let events = result.unwrap_or_default();
            futures::stream::iter(events)
        });

        Ok(flatten.boxed())
    }
}

fn build_chat_body(request: &LLMRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();

    if let Some(system) = &request.system {
        messages.push(json!({"role": "system", "content": system}));
    }

    for msg in &request.messages {

        match msg.role {
            Role::System => {
                let text = msg.content.iter().filter_map(|p| {
                    if let ContentPart::Text { text } = p { Some(text.as_str()) } else { None }
                }).collect::<Vec<_>>().concat();
                messages.push(json!({"role": "system", "content": text}));
            }
            Role::Assistant => {
                let text_parts: Vec<&str> = msg
                    .content
                    .iter()
                    .filter_map(|p| {
                        if let ContentPart::Text { text } = p {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                let has_text = !text_parts.is_empty();

                let tool_calls: Vec<Value> = msg
                    .content
                    .iter()
                    .filter_map(|p| {
                        if let ContentPart::ToolCall { id, name, arguments } = p {
                            Some(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(arguments).unwrap_or_default()
                                }
                            }))
                        } else {
                            None
                        }
                    })
                    .collect();
                let has_tools = !tool_calls.is_empty();

                let mut m = json!({"role": "assistant"});
                if has_text {
                    m["content"] = json!(text_parts.concat());
                }
                if has_tools {
                    m["tool_calls"] = json!(tool_calls);
                }
                if !has_text && !has_tools {
                    m["content"] = json!("");
                }
                messages.push(m);
            }
            Role::Tool => {
                for part in &msg.content {
                    if let ContentPart::ToolResult { tool_call_id, content } = part {
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_call_id,
                            "content": content
                        }));
                    }
                }
            }
            Role::User => {
                let texts: Vec<&str> = msg
                    .content
                    .iter()
                    .filter_map(|p| {
                        if let ContentPart::Text { text } = p {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                messages.push(json!({"role": "user", "content": texts.concat()}));
            }
        }
    }

    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema
                }
            })
        })
        .collect();

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "tools": tools,
    });

    if let Some(max_tokens) = request.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    body
}
fn parse_sse_events_from_buf(
    buf: &mut String,
    tool_ids: &mut std::collections::HashMap<u64, String>,
) -> Vec<LLMEvent> {
    let mut events = Vec::new();

    loop {
        let newline_pos = match buf.find('\n') {
            Some(pos) => pos,
            None => break,
        };
        let line = buf[..newline_pos].to_string();
        buf.drain(..=newline_pos);

        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim_end_matches('\r');
            if data == "[DONE]" {
                continue;
            }
            let Ok(val) = serde_json::from_str::<Value>(data) else {
                continue;
            };

            if let Some(delta) = val["choices"][0]["delta"]["content"].as_str() {
                if !delta.is_empty() {
                    events.push(LLMEvent::Text {
                        delta: delta.to_string(),
                    });
                }
            }

            if let Some(reasoning) = val["choices"][0]["delta"]["reasoning_content"].as_str() {
                if !reasoning.is_empty() {
                    events.push(LLMEvent::Reasoning {
                        delta: reasoning.to_string(),
                    });
                }
            }

            if let Some(tcs) = val["choices"][0]["delta"]["tool_calls"].as_array() {
                for tc in tcs {
                    let index = tc["index"].as_u64().unwrap_or(0);
                    if let Some(id) = tc["id"].as_str().filter(|s| !s.is_empty()) {
                        let name = tc["function"]["name"].as_str().unwrap_or("");
                        tool_ids.insert(index, id.to_string());
                        if !name.is_empty() {
                            events.push(LLMEvent::ToolCallStart {
                                id: id.to_string(),
                                name: name.to_string(),
                            });
                        }
                    }
                    if let Some(arg_delta) = tc["function"]["arguments"].as_str() {
                        if !arg_delta.is_empty() {
                            let id = tool_ids
                                .get(&index)
                                .cloned()
                                .unwrap_or_default();
                            events.push(LLMEvent::ToolCallDelta {
                                id,
                                delta: arg_delta.to_string(),
                            });
                        }
                    }
                }
            }

            if let Some(finish) = val["choices"][0]["finish_reason"].as_str() {
                let reason = match finish {
                    "stop" => FinishReason::Stop,
                    "tool_calls" => FinishReason::ToolCalls,
                    "length" => FinishReason::Length,
                    _ => FinishReason::Stop,
                };
                let usage = val["usage"].as_object().map(|u| Usage {
                    prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                    completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
                });
                events.push(LLMEvent::Finish {
                    finish_reason: reason,
                    usage,
                });
            }
        }
    }

    events
}
