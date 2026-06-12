use super::{Tool, ToolContext, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Command;

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents using regular expressions"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for"
                },
                "include": {
                    "type": ["string", "null"],
                    "description": "File pattern to include (glob)"
                },
                "path": {
                    "type": ["string", "null"],
                    "description": "Directory to search in (default: cwd)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' argument"))?;
        let path = args["path"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| ctx.cwd.clone());

        let mut cmd = Command::new("rg");
        cmd.arg("--line-number")
            .arg("--with-filename")
            .arg("--color")
            .arg("never")
            .current_dir(&path);

        if let Some(include) = args["include"].as_str() {
            cmd.arg("--glob").arg(include);
        }

        cmd.arg(pattern);

        let output = cmd.output().context("Failed to run ripgrep")?;

        let result = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr);

        let output = if result.is_empty() && !stderr.is_empty() {
            format!("No matches found.\n{}", stderr)
        } else if result.is_empty() {
            "No matches found.".to_string()
        } else {
            result.trim().to_string()
        };

        let line_count = output.lines().count();

        Ok(ToolResult {
            title: format!("grep: {}", pattern),
            output,
            metadata: json!({"pattern": pattern, "path": path, "matches": line_count}),
        })
    }
}
