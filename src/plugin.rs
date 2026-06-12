use crate::config::PluginConfig;
use crate::tools::{Tool, ToolContext, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use tokio::process::Command;

/// A plugin adds custom tools and slash commands to the session.
#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn tools(&self) -> Vec<Box<dyn Tool>>;
    async fn handle_command(&self, _cmd: &str, _args: &[&str]) -> Option<String> {
        None
    }
}

/// A plugin that delegates to an external process for tool execution.
pub struct CommandPlugin {
    name: String,
    description: String,
    tool_name: String,
    tool_description: String,
    command: String,
    args: Vec<String>,
}

#[async_trait]
impl Plugin for CommandPlugin {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ProcessTool {
            name: self.tool_name.clone(),
            description: self.tool_description.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
        })]
    }
}

pub struct ProcessTool {
    name: String,
    description: String,
    command: String,
    args: Vec<String>,
}

#[async_trait]
impl Tool for ProcessTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Input to the plugin tool"
                }
            },
            "required": ["input"]
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let input = args["input"].as_str().unwrap_or("");
        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args);
        cmd.arg(input);
        let output = cmd.output().await?;
        let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();
        let combined = if stderr_str.is_empty() {
            stdout_str.clone()
        } else {
            format!("{}\nstderr:\n{}", stdout_str, stderr_str)
        };
        Ok(ToolResult {
            title: format!("plugin:{}", self.name),
            output: combined,
            metadata: serde_json::json!({
                "exit_code": output.status.code(),
                "stdout": stdout_str,
                "stderr": stderr_str,
            }),
        })
    }
}

pub struct PluginManager {
    plugins: Vec<Box<dyn Plugin>>,
    tool_to_plugin: HashMap<String, usize>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            tool_to_plugin: HashMap::new(),
        }
    }

    pub fn load_from_config(config: &HashMap<String, PluginConfig>) -> Self {
        let mut mgr = Self::new();
        for (name, cfg) in config {
            let tool_cfg = match &cfg.tool {
                Some(t) => t,
                None => continue,
            };
            let tool_name = format!("plugin_{}", name);
            let plugin = CommandPlugin {
                name: name.clone(),
                description: cfg.description.clone().unwrap_or_default(),
                tool_name,
                tool_description: tool_cfg.description.clone().unwrap_or_default(),
                command: tool_cfg.command.clone(),
                args: tool_cfg.args.clone().unwrap_or_default(),
            };
            mgr.register(Box::new(plugin));
        }
        mgr
    }

    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        let idx = self.plugins.len();
        for tool in plugin.tools() {
            self.tool_to_plugin.insert(tool.name().to_string(), idx);
        }
        self.plugins.push(plugin);
    }

    pub fn tools(&self) -> Vec<Box<dyn Tool>> {
        let mut all = Vec::new();
        for plugin in &self.plugins {
            for tool in plugin.tools() {
                all.push(tool);
            }
        }
        all
    }

    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}
