use super::{Tool, ToolContext, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct SkillTool;

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        "Load a specialized skill that provides instructions and workflows for specific tasks"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the skill to load"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let name = args["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'name' argument"))?;

        let skill_dir = find_skill_dir(name, ctx)?;
        let skill_md = skill_dir.join("SKILL.md");
        let content = if skill_md.exists() {
            let text = std::fs::read_to_string(&skill_md)
                .context("Failed to read SKILL.md")?;
            text
        } else {
            anyhow::bail!("Skill '{}' not found at {}", name, skill_dir.display());
        };

        let mut file_info = String::new();
        let mut entries: Vec<_> = std::fs::read_dir(&skill_dir)
            .context("Failed to read skill directory")?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries.iter().take(10) {
            let fname = entry.file_name().to_string_lossy().to_string();
            let kind = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                format!("{}/", fname)
            } else {
                fname
            };
            file_info.push_str(&kind);
            file_info.push('\n');
        }
        if entries.len() > 10 {
            file_info.push_str(&format!("... and {} more files\n", entries.len() - 10));
        }

        let output = format!(
            "Skill: {}\nDirectory: {}\n\n{}\n\nFiles:\n{}",
            name,
            skill_dir.display(),
            content,
            file_info
        );

        Ok(ToolResult {
            title: format!("Loaded skill: {}", name),
            output,
            metadata: json!({"name": name, "directory": skill_dir.to_string_lossy()}),
        })
    }
}

fn find_skill_dir(name: &str, ctx: &ToolContext) -> Result<std::path::PathBuf> {
    let config = ctx.config.as_ref();

    let search_dirs: Vec<std::path::PathBuf> = if config.is_some() {
        let mut dirs = Vec::new();
        if let Some(opencode_dir) = dirs::config_dir() {
            dirs.push(opencode_dir.join("opencode").join("skills"));
        }
        if let Ok(cwd) = std::env::current_dir() {
            let local = cwd.join(".opencode").join("skills");
            if local.exists() {
                dirs.push(local);
            }
        }
        dirs
    } else {
        Vec::new()
    };

    for dir in &search_dirs {
        if dir.exists() {
            let skill_path = dir.join(name);
            if skill_path.exists() && skill_path.is_dir() {
                return Ok(skill_path);
            }
        }
    }

    anyhow::bail!("Skill '{}' not found. Looked in: {:?}", name, search_dirs);
}
