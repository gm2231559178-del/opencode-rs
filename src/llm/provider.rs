use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub system: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_call")]
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PermissionAction {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Text { delta: String },
    ToolCall { id: String, name: String, arguments: Value },
    ToolResult { id: String, name: String, output: String },
    PermissionRequest { request_id: String, tool_name: String, args: Value },
    Done { response: String },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LLMEvent {
    #[serde(rename = "text")]
    Text { delta: String },
    #[serde(rename = "tool_call_start")]
    ToolCallStart { id: String, name: String },
    #[serde(rename = "tool_call_delta")]
    ToolCallDelta { id: String, delta: String },
    #[serde(rename = "finish")]
    Finish {
        finish_reason: FinishReason,
        usage: Option<Usage>,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn generate(&self, request: &LLMRequest) -> Result<LLMResponse>;
    async fn stream(&self, request: &LLMRequest) -> Result<BoxStream<'static, LLMEvent>>;
}
