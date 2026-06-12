use super::{Tool, ToolContext, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a new file (will overwrite existing files)"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let file_path = args["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'file_path' argument"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' argument"))?;

        let path = std::path::Path::new(file_path);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent directory for {}", file_path))?;
        }

        tracing::debug!(file = %file_path, bytes = %content.len(), "write: writing");
        let existed = path.exists();
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write file: {}", file_path))?;
        tracing::debug!(file = %file_path, existed = %existed, "write: done");

        let action = if existed { "Overwritten" } else { "Written" };

        Ok(ToolResult {
            title: format!("{}: {}", action, file_path),
            output: format!("{} {} bytes to {}", action, content.len(), file_path),
            metadata: json!({"file_path": file_path, "bytes": content.len(), "existed": existed}),
        })
    }
}
