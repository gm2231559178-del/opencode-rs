use super::{Tool, ToolContext, ToolResult};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a structured patch to files in the codebase. Supports adding new files, updating existing files, and deleting files."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patchText": {
                    "type": "string",
                    "description": "The full patch text in a structured patch format. Each hunk starts with a file path marker (# <filepath>), followed by lines with + (add), - (delete), or no prefix (context)."
                }
            },
            "required": ["patchText"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let patch_text = args["patchText"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'patchText' argument"))?;

        let hunks = parse_patch(patch_text)?;
        let mut applied = Vec::new();

        for hunk in &hunks {
            let file_path = if std::path::Path::new(&hunk.file_path).is_absolute() {
                std::path::PathBuf::from(&hunk.file_path)
            } else {
                std::path::Path::new(&ctx.cwd).join(&hunk.file_path)
            };

            match hunk.operation {
                HunkOp::Add => {
                    if file_path.exists() {
                        applied.push(format!("SKIP (exists): {}", hunk.file_path));
                        continue;
                    }
                    if let Some(parent) = file_path.parent() {
                        std::fs::create_dir_all(parent)
                            .context("Failed to create parent directories")?;
                    }
                    let content = hunk.lines.iter().map(|l| &l.text[..]).collect::<Vec<_>>().join("\n");
                    std::fs::write(&file_path, &content)
                        .with_context(|| format!("Failed to write {}", file_path.display()))?;
                    applied.push(format!("ADDED: {}", hunk.file_path));
                }
                HunkOp::Update => {
                    if !file_path.exists() {
                        applied.push(format!("SKIP (not found): {}", hunk.file_path));
                        continue;
                    }
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("Failed to read {}", file_path.display()))?;
                    let new_content = apply_update_hunk(&content, &hunk.lines)?;
                    std::fs::write(&file_path, &new_content)
                        .with_context(|| format!("Failed to write {}", file_path.display()))?;
                    applied.push(format!("UPDATED: {}", hunk.file_path));
                }
                HunkOp::Delete => {
                    if !file_path.exists() {
                        applied.push(format!("SKIP (not found): {}", hunk.file_path));
                        continue;
                    }
                    std::fs::remove_file(&file_path)
                        .with_context(|| format!("Failed to delete {}", file_path.display()))?;
                    applied.push(format!("DELETED: {}", hunk.file_path));
                }
            }
        }

        let output = applied.join("\n");
        Ok(ToolResult {
            title: format!("Applied patch: {} hunks", hunks.len()),
            output,
            metadata: json!({"applied": applied, "count": hunks.len()}),
        })
    }
}

#[derive(Debug)]
enum HunkOp {
    Add,
    Update,
    Delete,
}

#[derive(Debug)]
struct Hunk {
    file_path: String,
    operation: HunkOp,
    lines: Vec<Line>,
}

#[derive(Debug)]
struct Line {
    op: char,
    text: String,
}

fn parse_patch(text: &str) -> Result<Vec<Hunk>> {
    let mut hunks = Vec::new();
    let mut current_hunk: Option<Hunk> = None;

    for line in text.lines() {
        if line.starts_with("# ") {
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            let rest = &line[2..];
            let (file_path, op_str) = rest.split_once(':').unwrap_or((rest, "update"));
            let operation = match op_str.trim() {
                "add" => HunkOp::Add,
                "delete" => HunkOp::Delete,
                _ => HunkOp::Update,
            };
            current_hunk = Some(Hunk {
                file_path: file_path.trim().to_string(),
                operation,
                lines: Vec::new(),
            });
        } else if let Some(ref mut hunk) = current_hunk {
            if line.starts_with("+") {
                hunk.lines.push(Line { op: '+', text: line[1..].to_string() });
            } else if line.starts_with("-") {
                hunk.lines.push(Line { op: '-', text: line[1..].to_string() });
            } else {
                hunk.lines.push(Line { op: ' ', text: line.to_string() });
            }
        }
    }
    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    if hunks.is_empty() {
        bail!("No valid hunks found in patch text. Expected format: '# <filepath>' headers followed by +/- lines.");
    }
    Ok(hunks)
}

fn apply_update_hunk(original: &str, lines: &[Line]) -> Result<String> {
    let original_lines: Vec<&str> = original.lines().collect();
    let mut result: Vec<String> = Vec::new();
    let mut i = 0;

    while i < original_lines.len() {
        let mut matched = false;
        if i + lines.len() <= original_lines.len() {
            let mut all_match = true;
            for (j, l) in lines.iter().enumerate() {
                match l.op {
                    '+' => {}
                    '-' => {
                        if original_lines[i + j] != l.text {
                            all_match = false;
                            break;
                        }
                    }
                    ' ' => {
                        if original_lines[i + j] != l.text {
                            all_match = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if all_match {
                for l in lines {
                    if l.op != '-' {
                        result.push(l.text.clone());
                    }
                }
                i += lines.iter().filter(|l| l.op != '+').count();
                matched = true;
            }
        }
        if !matched {
            result.push(original_lines[i].to_string());
            i += 1;
        }
    }

    Ok(result.join("\n"))
}
