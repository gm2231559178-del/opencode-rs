use super::{Tool, ToolContext, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    pub priority: Option<String>,
}

pub struct TodowriteTool {
    pub todos: Arc<Mutex<Vec<TodoItem>>>,
}

impl TodowriteTool {
    pub fn new() -> Self {
        Self {
            todos: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn new_shared(todos: Arc<Mutex<Vec<TodoItem>>>) -> Self {
        Self { todos }
    }
}

#[async_trait]
impl Tool for TodowriteTool {
    fn name(&self) -> &str {
        "todowrite"
    }

    fn description(&self) -> &str {
        "Create and maintain a structured task list for the current coding session. Tracks progress, organizes multi-step work, and surfaces status to the user."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "Brief description of the task"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "cancelled"],
                                "description": "Current status of the task"
                            },
                            "priority": {
                                "type": ["string", "null"],
                                "enum": ["high", "medium", "low"],
                                "description": "Priority level of the task"
                            }
                        },
                        "required": ["content", "status"]
                    },
                    "description": "The updated todo list (replaces all existing items)"
                }
            },
            "required": ["todos"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let items: Vec<TodoItem> = serde_json::from_value(args["todos"].clone())
            .context("Invalid 'todos' format")?;

        let mut todos = self.todos.lock().await;
        *todos = items.clone();

        let mut output = String::from("Task list updated:\n");
        for item in &items {
            let priority_str = item
                .priority
                .as_deref()
                .map(|p| format!(" [{}]", p))
                .unwrap_or_default();
            output.push_str(&format!("  [{}]{} {}\n", item.status, priority_str, item.content));
        }

        Ok(ToolResult {
            title: "Todo list updated".to_string(),
            output,
            metadata: json!({"todo_count": items.len()}),
        })
    }
}
