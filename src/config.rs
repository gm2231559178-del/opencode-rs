use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub provider: HashMap<String, ProviderConfig>,
    pub model: Option<String>,
    #[serde(default)]
    pub permission: PermissionConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    pub shell: Option<String>,
    pub instructions: Option<Vec<String>>,
    pub username: Option<String>,
    #[serde(default)]
    pub agent: HashMap<String, AgentConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionConfig {
    #[serde(default)]
    pub auto_approve: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub enabled: Vec<String>,
    #[serde(default)]
    pub disabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    pub model: Option<String>,
    pub instructions: Option<Vec<String>>,
}

pub fn load_config() -> Result<Config> {
    let paths = config_paths();
    for path in &paths {
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config: {}", path.display()))?;
            let content = strip_jsonc_comments(&content);
            let config: Config = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse config: {}", path.display()))?;
            tracing::info!("Loaded config from {}", path.display());
            return Ok(config);
        }
    }
    tracing::info!("No config found, using defaults");
    Ok(Config::default())
}

fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("opencode").join("opencode.jsonc"));
        paths.push(config_dir.join("opencode").join("opencode.json"));
        paths.push(config_dir.join("opencode-rs").join("opencode.jsonc"));
        paths.push(config_dir.join("opencode-rs").join("opencode.json"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".opencode").join("opencode.jsonc"));
        paths.push(cwd.join(".opencode").join("opencode.json"));
    }
    if let Ok(path) = std::env::var("OPENCODE_CONFIG") {
        paths.push(PathBuf::from(path));
    }
    paths
}

fn strip_jsonc_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '/' {
            if chars[i + 1] == '/' {
                i += 2;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if chars[i + 1] == '*' {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                if i + 1 < chars.len() {
                    i += 2;
                }
                continue;
            }
        }
        if chars[i] == '"' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            result.push_str(&input[start..i]);
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}
