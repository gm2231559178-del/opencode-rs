pub mod bash;
pub mod edit;
pub mod glob_tool;
pub mod grep_tool;
pub mod read;
pub mod task;
pub mod write;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

pub struct ToolContext {
    pub session_id: String,
    pub message_id: String,
    pub cwd: String,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub title: String,
    pub output: String,
    pub metadata: Value,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult>;
}

pub fn builtin_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(bash::BashTool),
        Box::new(read::ReadTool),
        Box::new(write::WriteTool),
        Box::new(edit::EditTool),
        Box::new(grep_tool::GrepTool),
        Box::new(glob_tool::GlobTool),
        Box::new(task::TaskTool),
    ]
}
