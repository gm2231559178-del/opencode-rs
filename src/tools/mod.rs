pub mod apply_patch;
pub mod bash;
pub mod edit;
pub mod glob_tool;
pub mod grep_tool;
pub mod question;
pub mod read;
pub mod skill;
pub mod task;
pub mod todowrite;
pub mod webfetch;
pub mod websearch;
pub mod write;

use crate::config::Config;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct ToolContext {
    pub session_id: String,
    pub message_id: String,
    pub cwd: String,
    pub config: Option<Arc<Config>>,
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
        Box::new(apply_patch::ApplyPatchTool),
        Box::new(bash::BashTool),
        Box::new(edit::EditTool),
        Box::new(glob_tool::GlobTool),
        Box::new(grep_tool::GrepTool),
        Box::new(question::QuestionTool),
        Box::new(read::ReadTool),
        Box::new(skill::SkillTool),
        Box::new(task::TaskTool),
        Box::new(todowrite::TodowriteTool::new()),
        Box::new(webfetch::WebfetchTool),
        Box::new(websearch::WebsearchTool),
        Box::new(write::WriteTool),
    ]
}
