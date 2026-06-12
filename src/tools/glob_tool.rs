use super::{Tool, ToolContext, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Command;

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match (e.g. **/*.rs)"
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

        let mut cmd = Command::new("fd");
        cmd.arg("--glob").arg(pattern).current_dir(&path);

        let output = cmd.output().context("Failed to run fd")?;
        let result = String::from_utf8_lossy(&output.stdout).to_string();

        let files: Vec<&str> = result.lines().filter(|l| !l.is_empty()).collect();
        let output = if files.is_empty() {
            format!("No files matching '{}'", pattern)
        } else {
            let max_display = 200;
            let mut out = String::new();
            for (i, f) in files.iter().enumerate() {
                if i >= max_display {
                    out.push_str(&format!(
                        "... and {} more files\n",
                        files.len() - max_display
                    ));
                    break;
                }
                out.push_str(f);
                out.push('\n');
            }
            if max_display >= files.len() {
                out.push_str(&format!("Found {} files", files.len()));
            }
            out
        };

        Ok(ToolResult {
            title: format!("glob: {}", pattern),
            output,
            metadata: json!({"pattern": pattern, "path": path, "count": files.len()}),
        })
    }
}
