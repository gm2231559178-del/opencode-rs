use super::{Tool, ToolContext, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

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
                    "description": "Type of agent (explore, general)",
                    "default": "general"
                }
            },
            "required": ["description", "prompt"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let description = args["description"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'description' argument"))?;
        let prompt = args["prompt"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' argument"))?;
        let _subagent = args["subagent_type"].as_str().unwrap_or("general");

        // For v1, task tool just echoes the intent
        // In a full implementation, this would spawn a sub-agent
        let output = format!(
            "Task delegated: {}\n\nPrompt:\n{}\n\n(Sub-agent execution not yet implemented in v1)",
            description, prompt
        );

        Ok(ToolResult {
            title: format!("Task: {}", description),
            output,
            metadata: json!({"description": description, "subagent_type": _subagent}),
        })
    }
}
