use super::{Tool, ToolContext, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact string match with new content"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The text to replace it with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences",
                    "default": false
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let file_path = args["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'file_path' argument"))?;
        let old_string = args["old_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'old_string' argument"))?;
        let new_string = args["new_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'new_string' argument"))?;
        let replace_all = args["replace_all"].as_bool().unwrap_or(false);

        let path = std::path::Path::new(file_path);
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", file_path))?;

        let count = if content.contains(old_string) {
            content.matches(old_string).count()
        } else {
            0
        };

        if count == 0 {
            anyhow::bail!("old_string not found in {}", file_path);
        }

        if count > 1 && !replace_all {
            anyhow::bail!(
                "Found {} matches for old_string in {}. Use replace_all to replace all.",
                count,
                file_path
            );
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        tracing::debug!(
            file = %file_path,
            matches = %count,
            replace_all = %replace_all,
            old_len = %old_string.len(),
            new_len = %new_string.len(),
            "edit: replacing"
        );

        std::fs::write(path, &new_content)
            .with_context(|| format!("Failed to write file: {}", file_path))?;
        tracing::debug!(file = %file_path, "edit: done");

        let occurrences = if replace_all { "all" } else { "first" };

        Ok(ToolResult {
            title: format!("Edited: {}", file_path),
            output: format!(
                "Replaced {} occurrence(s) of old_string with new_string in {}",
                occurrences, file_path
            ),
            metadata: json!({
                "file_path": file_path,
                "replacements": if replace_all { count } else { 1 }
            }),
        })
    }
}
