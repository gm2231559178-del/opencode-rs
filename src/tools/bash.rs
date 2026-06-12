use super::{Tool, ToolContext, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::process::Command;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command in the user's environment"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout": {
                    "type": ["number", "null"],
                    "description": "Timeout in milliseconds (default: 120000)"
                },
                "workdir": {
                    "type": ["string", "null"],
                    "description": "Working directory (default: cwd)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;
        let timeout_ms = args["timeout"].as_u64().unwrap_or(120000);
        let workdir = args["workdir"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| ctx.cwd.clone());

        tracing::debug!(command = %command, workdir = %workdir, timeout = %timeout_ms, "bash: executing");
        let output = run_shell(command, &workdir, timeout_ms).await?;
        tracing::debug!(output_len = %output.len(), "bash: completed");

        Ok(ToolResult {
            title: format!("$ {}", command),
            output,
            metadata: json!({"command": command, "workdir": workdir}),
        })
    }
}

async fn run_shell(command: &str, workdir: &str, timeout_ms: u64) -> Result<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = Command::new(&shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(workdir)
        .kill_on_drop(true);

    let result = tokio::time::timeout(Duration::from_millis(timeout_ms), cmd.output()).await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = if stderr.is_empty() {
                stdout.to_string()
            } else {
                format!("{}\n{}", stdout, stderr)
            };
            Ok(combined.trim().to_string())
        }
        Ok(Err(e)) => Err(anyhow::anyhow!("Command failed: {}", e)),
        Err(_) => Err(anyhow::anyhow!("Command timed out after {}ms", timeout_ms)),
    }
}
