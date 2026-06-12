use super::{Tool, ToolContext, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file or directory"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file or directory"
                },
                "offset": {
                    "type": ["integer", "null"],
                    "description": "Line number to start from (1-indexed)"
                },
                "limit": {
                    "type": ["integer", "null"],
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let file_path = args["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'file_path' argument"))?;

        let path = std::path::Path::new(file_path);
        if !path.exists() {
            anyhow::bail!("Path does not exist: {}", file_path);
        }

        tracing::debug!(file = %file_path, "read: opening");

        let output = if path.is_dir() {
            read_directory(path)?
        } else {
            let offset = args["offset"].as_u64().unwrap_or(0) as usize;
            let limit = args["limit"].as_u64().map(|l| l as usize);
            read_file(path, offset, limit)?
        };

        Ok(ToolResult {
            title: format!("Read: {}", file_path),
            output,
            metadata: json!({"file_path": file_path}),
        })
    }
}

fn read_directory(path: &std::path::Path) -> Result<String> {
    let mut entries: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(path).context("Failed to read directory")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let kind = if entry.file_type()?.is_dir() {
            format!("{}/", name)
        } else {
            name
        };
        entries.push(kind);
    }
    entries.sort();
    let max_display = 200;
    let mut output = String::new();
    for (i, entry) in entries.iter().enumerate() {
        if i >= max_display {
            output.push_str(&format!("... and {} more entries\n", entries.len() - max_display));
            break;
        }
        output.push_str(entry);
        output.push('\n');
    }
    Ok(output.trim_end().to_string())
}

fn read_file(path: &std::path::Path, offset: usize, limit: Option<usize>) -> Result<String> {
    let content = std::fs::read_to_string(path).context("Failed to read file")?;
    let lines: Vec<&str> = content.lines().collect();
    let start = if offset > 0 { offset - 1 } else { 0 };
    let end = match limit {
        Some(l) => (start + l).min(lines.len()),
        None => lines.len(),
    };

    let mut output = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        output.push_str(&format!("{}: {}\n", start + i + 1, line));
    }
    Ok(output.trim_end().to_string())
}
