use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerConfig {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub url: Option<String>,
    pub transport: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginToolConfig {
    pub command: String,
    pub args: Option<Vec<String>>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginConfig {
    pub description: Option<String>,
    pub tool: Option<PluginToolConfig>,
}

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
    #[serde(default)]
    pub mcp: HashMap<String, McpServerConfig>,
    #[serde(default)]
    pub plugin: HashMap<String, PluginConfig>,
}

pub fn load_config() -> Result<Config> {
    // Layered config merging: global -> project -> OPENCODE_CONFIG env var
    let mut config = Config::default();

    for path in layered_config_paths() {
        if !path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let content = strip_jsonc_comments(&content);
        match serde_json::from_str::<Config>(&content) {
            Ok(layer) => {
                config = merge_config(config, layer);
                tracing::info!("Loaded config layer from {}", path.display());
            }
            Err(e) => {
                tracing::warn!("Skipping config {}: {}", path.display(), e);
            }
        }
    }

    // Apply environment variable overrides
    if let Ok(val) = std::env::var("OPENCODE_MODEL") {
        config.model = Some(val);
    }
    if let Ok(val) = std::env::var("OPENCODE_PROVIDER_API_KEY") {
        if let Some(ref model) = config.model {
            let provider_name = model.split('/').next().unwrap_or("openai");
            config
                .provider
                .entry(provider_name.to_string())
                .or_default()
                .api_key = Some(val);
        } else {
            config
                .provider
                .entry("openai".to_string())
                .or_default()
                .api_key = Some(val);
        }
    }
    if let Ok(val) = std::env::var("OPENCODE_PROVIDER_BASE_URL") {
        if let Some(ref model) = config.model {
            let provider_name = model.split('/').next().unwrap_or("openai");
            config
                .provider
                .entry(provider_name.to_string())
                .or_default()
                .base_url = Some(val);
        }
    }
    if let Ok(val) = std::env::var("OPENCODE_SHELL") {
        config.shell = Some(val);
    }
    if let Ok(val) = std::env::var("OPENCODE_USERNAME") {
        config.username = Some(val);
    }
    if let Ok(val) = std::env::var("OPENCODE_AUTO_APPROVE") {
        config.permission.auto_approve = val.split(',').map(|s| s.trim().to_string()).collect();
    }
    if let Ok(val) = std::env::var("OPENCODE_DENY") {
        config.permission.deny = val.split(',').map(|s| s.trim().to_string()).collect();
    }

    tracing::info!("Config loaded with layered merging and env var overrides");
    Ok(config)
}

fn layered_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Global configs (lowest priority)
    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("opencode").join("opencode.jsonc"));
        paths.push(config_dir.join("opencode").join("opencode.json"));
        paths.push(config_dir.join("opencode-rs").join("opencode.jsonc"));
        paths.push(config_dir.join("opencode-rs").join("opencode.json"));
    }

    // Project-level configs (medium priority)
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".opencode").join("opencode.jsonc"));
        paths.push(cwd.join(".opencode").join("opencode.json"));
    }

    // OPENCODE_CONFIG env var (highest file priority)
    if let Ok(path) = std::env::var("OPENCODE_CONFIG") {
        paths.push(PathBuf::from(path));
    }

    paths
}

fn merge_config(base: Config, overlay: Config) -> Config {
    Config {
        model: overlay.model.or(base.model),
        shell: overlay.shell.or(base.shell),
        username: overlay.username.or(base.username),
        instructions: overlay.instructions.or(base.instructions),
        permission: PermissionConfig {
            auto_approve: if overlay.permission.auto_approve.is_empty() {
                base.permission.auto_approve
            } else {
                overlay.permission.auto_approve
            },
            deny: if overlay.permission.deny.is_empty() {
                base.permission.deny
            } else {
                overlay.permission.deny
            },
        },
        tools: ToolsConfig {
            enabled: if overlay.tools.enabled.is_empty() {
                base.tools.enabled
            } else {
                overlay.tools.enabled
            },
            disabled: if overlay.tools.disabled.is_empty() {
                base.tools.disabled
            } else {
                overlay.tools.disabled
            },
        },
        provider: {
            let mut merged = base.provider;
            for (key, val) in overlay.provider {
                merged.insert(key, val);
            }
            merged
        },
        agent: {
            let mut merged = base.agent;
            for (key, val) in overlay.agent {
                merged.insert(key, val);
            }
            merged
        },
        mcp: {
            let mut merged = base.mcp;
            for (key, val) in overlay.mcp {
                merged.insert(key, val);
            }
            merged
        },
        plugin: {
            let mut merged = base.plugin;
            for (key, val) in overlay.plugin {
                merged.insert(key, val);
            }
            merged
        },
    }
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

pub fn config_set(key: &str, value: &str) -> Result<String> {
    let cwd = std::env::current_dir()?;
    let config_dir = cwd.join(".opencode");
    let config_path = config_dir.join("opencode.jsonc");

    let mut config: Config = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&strip_jsonc_comments(&content))?
    } else {
        Config::default()
    };

    match key {
        "model" => config.model = Some(value.to_string()),
        "shell" => config.shell = Some(value.to_string()),
        "username" => config.username = Some(value.to_string()),
        "instructions" => {
            config.instructions = Some(vec![value.to_string()]);
        }
        "auto_approve" => {
            config.permission.auto_approve = value.split(',').map(|s| s.trim().to_string()).collect();
        }
        "deny" => {
            config.permission.deny = value.split(',').map(|s| s.trim().to_string()).collect();
        }
        _ => {
            // Try provider.* or mcp.* or plugin.* or agent.* keys
            if let Some(rest) = key.strip_prefix("provider.") {
                if let Some((provider_name, field)) = rest.split_once('.') {
                    let entry = config.provider.entry(provider_name.to_string()).or_default();
                    match field {
                        "api_key" => entry.api_key = Some(value.to_string()),
                        "base_url" => entry.base_url = Some(value.to_string()),
                        "default_model" => entry.default_model = Some(value.to_string()),
                        _ => return Ok(format!("Unknown provider field: {}", field)),
                    }
                } else {
                    config
                        .provider
                        .entry(rest.to_string())
                        .or_default()
                        .default_model = Some(value.to_string());
                }
            } else {
                return Ok(format!("Unknown config key: {}. Supported keys: model, shell, username, instructions, auto_approve, deny, provider.<name>.api_key, provider.<name>.base_url", key));
            }
        }
    }

    std::fs::create_dir_all(&config_dir)?;

    let output = serde_json::to_string_pretty(&config)?;
    // Add jsonc header
    let header = "// opencode-rs configuration\n// See https://opencode.ai for documentation\n\n";
    std::fs::write(&config_path, format!("{}{}", header, output))?;

    Ok(format!("Set {} = {} in {}", key, value, config_path.display()))
}
