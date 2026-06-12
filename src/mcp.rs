use crate::config::McpServerConfig;
use crate::tools::{Tool, ToolContext, ToolResult};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

const JSON_RPC_VERSION: &str = "2.0";

struct McpConnection {
    child: Child,
    stdin: tokio::io::BufWriter<tokio::process::ChildStdin>,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

impl McpConnection {
    async fn connect(command: &str, args: &[String]) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        Ok(Self {
            child,
            stdin: tokio::io::BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let request = serde_json::json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        let mut response_line = String::new();
        self.stdout.read_line(&mut response_line).await?;

        let response: Value = serde_json::from_str(&response_line)?;
        if let Some(error) = response.get("error") {
            bail!("MCP error: {}", error);
        }
        Ok(response["result"].clone())
    }
}

#[derive(Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub server_name: String,
    connection: std::sync::Arc<Mutex<McpConnection>>,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        self.input_schema.clone()
    }
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let mut conn = self.connection.lock().await;
        match conn.send_request("tools/call", serde_json::json!({
            "name": self.name,
            "arguments": args,
        })).await {
            Ok(result) => {
                let content = result["content"].as_array().map(|arr| {
                    arr.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
                }).unwrap_or_default();
                Ok(ToolResult {
                    title: format!("mcp:{}", self.name),
                    output: content,
                    metadata: result,
                })
            }
            Err(e) => Ok(ToolResult {
                title: format!("mcp:{} error", self.name),
                output: format!("Error: {}", e),
                metadata: Value::Null,
            }),
        }
    }
}

pub async fn connect_mcp_servers(
    servers: &HashMap<String, McpServerConfig>,
) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    for (name, config) in servers {
        let (command, args) = match (&config.command, &config.url) {
            (Some(cmd), _) => (cmd.clone(), config.args.clone().unwrap_or_default()),
            (None, Some(_url)) => {
                tracing::warn!("MCP server '{}' uses URL transport (not yet supported)", name);
                continue;
            }
            (None, None) => {
                tracing::warn!("MCP server '{}' has no command or url", name);
                continue;
            }
        };

        let mut conn = match McpConnection::connect(&command, &args).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to connect to MCP server '{}': {}", name, e);
                continue;
            }
        };

        let result = match conn.send_request("tools/list", serde_json::json!({})).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Failed to list tools from MCP server '{}': {}", name, e);
                continue;
            }
        };

        let conn_arc = std::sync::Arc::new(Mutex::new(conn));

        if let Some(tool_list) = result["tools"].as_array() {
            for tool_def in tool_list {
                let tool_name = format!("mcp_{}/{}", name, tool_def["name"].as_str().unwrap_or("unknown"));
                tools.push(Box::new(McpTool {
                    name: tool_name,
                    description: tool_def["description"].as_str().unwrap_or("").to_string(),
                    input_schema: tool_def["inputSchema"].clone(),
                    server_name: name.clone(),
                    connection: conn_arc.clone(),
                }));
            }
            tracing::info!("Connected to MCP server '{}' ({} tools)", name, tool_list.len());
        }
    }

    tools
}
