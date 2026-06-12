use super::{Tool, ToolContext, ToolResult};
use crate::config::Config;
use crate::llm::provider::{ContentPart, LLMRequest, Message, Role, ToolDef};
use crate::llm::{create_provider, default_model};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct TaskTool;

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Delegate a task to a sub-agent for parallel execution"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Short description of the task"
                },
                "prompt": {
                    "type": "string",
                    "description": "The detailed instructions for the sub-agent"
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Type of agent (explore=read-only, general=full access)",
                    "default": "general"
                }
            },
            "required": ["description", "prompt"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let description = args["description"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'description' argument"))?;
        let prompt = args["prompt"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' argument"))?;
        let subagent_type = args["subagent_type"].as_str().unwrap_or("general");

        let config = match &ctx.config {
            Some(c) => c.clone(),
            None => {
                return Ok(ToolResult {
                    title: format!("Task: {}", description),
                    output: "Sub-agent execution requires a config.".to_string(),
                    metadata: json!({"description": description, "subagent_type": subagent_type}),
                });
            }
        };

        match subagent_type {
            "explore" => {
                let result = run_subagent(config, prompt, true).await?;
                Ok(ToolResult {
                    title: format!("Explore: {}", description),
                    output: result,
                    metadata: json!({"description": description, "subagent_type": "explore"}),
                })
            }
            "general" | _ => {
                let result = run_subagent(config, prompt, false).await?;
                Ok(ToolResult {
                    title: format!("Task: {}", description),
                    output: result,
                    metadata: json!({"description": description, "subagent_type": "general"}),
                })
            }
        }
    }
}

async fn run_subagent(config: Arc<Config>, prompt: &str, read_only: bool) -> Result<String> {
    let model = config.model.clone().unwrap_or_else(|| "openai/gpt-4o".to_string());
    let provider_name = model.split('/').next().unwrap_or("openai").to_string();
    let model_id = model.splitn(2, '/').nth(1).unwrap_or_else(|| default_model(&provider_name)).to_string();

    let provider = create_provider(&config)?;

    let instructions = if read_only {
        "You are an explore sub-agent. Your job is to read files and explore the codebase to answer questions. Do NOT execute any commands, write files, or make edits. Use only read, grep, and glob tools.".to_string()
    } else {
        "You are a general sub-agent. You can use any available tool to complete your task. Be thorough and precise.".to_string()
    };

    let mut tools: Vec<Box<dyn super::Tool>> = vec![
        Box::new(super::read::ReadTool),
        Box::new(super::grep_tool::GrepTool),
        Box::new(super::glob_tool::GlobTool),
    ];
    if !read_only {
        tools.push(Box::new(super::bash::BashTool));
        tools.push(Box::new(super::write::WriteTool));
        tools.push(Box::new(super::edit::EditTool));
    }

    let (tools, tools_defs): (Vec<_>, Vec<_>) = tools.into_iter().map(|t| {
        let def = ToolDef {
            name: t.name().to_string(),
            description: t.description().to_string(),
            input_schema: t.parameters(),
        };
        (t, def)
    }).unzip();

    let request = LLMRequest {
        model: model_id,
        messages: vec![
            Message {
                role: Role::User,
                content: vec![ContentPart::Text { text: prompt.to_string() }],
            },
        ],
        tools: tools_defs,
        system: Some(instructions),
        max_tokens: Some(4096),
        temperature: None,
    };

    let response = provider.generate(&request).await?;

    let mut output = response.content.clone();

    for tc in &response.tool_calls {
        let tool = tools.iter().find(|t| t.name() == tc.name);
        if let Some(t) = tool {
            let ctx = ToolContext {
                session_id: String::new(),
                message_id: String::new(),
                cwd: std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                config: None,
            };
            let result = t.execute(tc.arguments.clone(), &ctx).await;
            match result {
                Ok(r) => {
                    output.push_str(&format!("\n\n--- {} ---\n{}", tc.name, r.output));
                }
                Err(e) => {
                    output.push_str(&format!("\n\n--- {} error: {} ---", tc.name, e));
                }
            }
        }
    }

    Ok(output)
}
