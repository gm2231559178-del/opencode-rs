use crate::config::Config;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "opencode-rs", version, about = "AI coding agent (Rust reimplementation)")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Provider (e.g. "openai", "anthropic")
    #[arg(long, global = true)]
    pub provider: Option<String>,

    /// Model (e.g. "openai/gpt-4o", "anthropic/claude-3-5-sonnet")
    #[arg(long, global = true)]
    pub model: Option<String>,

    /// Log level
    #[arg(long, default_value = "info", global = true)]
    pub log_level: String,

    /// Open a prompt immediately
    pub prompt: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the TUI
    Start {
        /// Working directory
        directory: Option<String>,
    },
    /// Run a single prompt (non-interactive)
    Run {
        /// The prompt to send
        prompt: Vec<String>,
    },
    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Print version
    Version,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show the current configuration
    Show,
    /// Set a config value
    Set {
        key: String,
        value: String,
    },
}

pub fn parse() -> Cli {
    Cli::parse()
}

pub fn merge_cli_config(config: &mut Config, cli: &Cli) {
    if let Some(provider) = &cli.provider {
        config.model = Some(format!(
            "{}/{}",
            provider,
            config
                .model
                .as_deref()
                .map(|m| m.split('/').nth(1).unwrap_or(m))
                .unwrap_or("gpt-4o")
        ));
    }
    if let Some(model) = &cli.model {
        config.model = Some(model.clone());
    }
}
