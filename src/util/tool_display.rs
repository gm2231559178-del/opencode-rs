use std::collections::HashMap;
use serde_json::Value;

fn tool_names() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("bash", "Run Command");
    m.insert("read", "Read File");
    m.insert("write", "Write File");
    m.insert("edit", "Edit File");
    m.insert("glob", "Search Files");
    m.insert("grep", "Search Content");
    m.insert("task", "Delegate Task");
    m.insert("websearch", "Web Search");
    m.insert("webfetch", "Fetch URL");
    m.insert("question", "Ask User");
    m.insert("skill", "Load Skill");
    m.insert("apply_patch", "Apply Patch");
    m.insert("todowrite", "Todo List");
    m
}

pub fn human_name(tool_name: &str) -> String {
    let names = tool_names();
    if let Some(name) = names.get(tool_name) {
        return name.to_string();
    }
    if tool_name.starts_with("mcp_") {
        return "MCP Tool".to_string();
    }
    if tool_name.starts_with("plugin_") {
        return "Plugin Tool".to_string();
    }
    tool_name.to_string()
}

pub fn tool_icon(tool_name: &str) -> &'static str {
    match tool_name {
        "bash" => "$ ",
        "read" => "\u{2192} ",  // →
        "write" => "\u{2190} ", // ←
        "edit" => "\u{2190} ",  // ←
        "apply_patch" => "%",
        "glob" => "\u{2731} ",  // ✱
        "grep" => "\u{2731} ",  // ✱
        "websearch" => "\u{25C6} ", // ◆
        "webfetch" => "%",
        "task" => "\u{2713} ",  // ✓
        "question" => "\u{2192} ", // →
        "skill" => "\u{2192} ",  // →
        "todowrite" => "\u{2699} ", // ⚙
        _ => "\u{2699} ", // ⚙ (generic)
    }
}

/// Format tool call arguments into a human-readable summary line.
/// Instead of dumping raw JSON, extract the key parameter for each tool type.
pub fn format_tool_args(tool_name: &str, args: &Value) -> String {
    match tool_name {
        "bash" => {
            args.get("command")
                .and_then(|c| c.as_str())
                .map(|c| c.to_string())
                .unwrap_or_default()
        }
        "read" => {
            args.get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|p| p.as_str())
                .map(|p| p.to_string())
                .unwrap_or_default()
        }
        "write" => {
            let path = args.get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|p| p.as_str())
                .unwrap_or("");

            let content_len = args.get("content")
                .and_then(|c| c.as_str())
                .map(|c| c.len())
                .unwrap_or(0);

            if content_len > 0 {
                format!("{} ({} chars)", path, content_len)
            } else {
                path.to_string()
            }
        }
        "edit" => {
            let path = args.get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|p| p.as_str())
                .unwrap_or("");

            let old_len = args.get("old_string")
                .and_then(|s| s.as_str())
                .map(|s| s.len())
                .unwrap_or(0);

            let new_len = args.get("new_string")
                .and_then(|s| s.as_str())
                .map(|s| s.len())
                .unwrap_or(0);

            if old_len > 0 || new_len > 0 {
                format!("{} ({}→{} chars)", path, old_len, new_len)
            } else {
                path.to_string()
            }
        }
        "glob" => {
            args.get("pattern")
                .and_then(|p| p.as_str())
                .map(|p| format!("`{}`", p))
                .unwrap_or_default()
        }
        "grep" => {
            let pattern = args.get("pattern")
                .and_then(|p| p.as_str())
                .unwrap_or("");

            let path = args.get("path")
                .or_else(|| args.get("include"))
                .and_then(|p| p.as_str())
                .unwrap_or("");

            if !path.is_empty() {
                format!("`{}` in {}", pattern, path)
            } else {
                format!("`{}`", pattern)
            }
        }
        "websearch" => {
            args.get("query")
                .and_then(|q| q.as_str())
                .map(|q| q.to_string())
                .unwrap_or_default()
        }
        "webfetch" => {
            args.get("url")
                .and_then(|u| u.as_str())
                .map(|u| u.to_string())
                .unwrap_or_default()
        }
        "question" => {
            args.get("question")
                .and_then(|q| q.as_str())
                .map(|q| q.to_string())
                .unwrap_or_default()
        }
        "task" => {
            let prompt = args.get("prompt")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let len = prompt.len();
            if len > 100 {
                let preview: String = prompt.chars().take(100).collect();
                format!("{}... ({} chars)", preview, len)
            } else if len > 0 {
                prompt.to_string()
            } else {
                String::new()
            }
        }
        "todowrite" => {
            args.get("action")
                .and_then(|a| a.as_str())
                .map(|a| format!("action: {}", a))
                .unwrap_or_default()
        }
        "apply_patch" => {
            let path = args.get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|p| p.as_str())
                .unwrap_or("");

            let patch_len = args.get("patch")
                .and_then(|p| p.as_str())
                .map(|p| p.len())
                .unwrap_or(0);

            if patch_len > 0 {
                format!("{} ({} line patch)", path, patch_len)
            } else {
                path.to_string()
            }
        }
        "skill" => {
            args.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.to_string())
                .unwrap_or_default()
        }
        _ if tool_name.starts_with("mcp_") => {
            let n = args.get("name")
                .or_else(|| args.get("tool"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !n.is_empty() {
                n.to_string()
            } else {
                // Show first string argument as a fallback
                args.as_object()
                    .and_then(|obj| obj.values().find_map(|v| v.as_str().map(|s| s.to_string())))
                    .unwrap_or_default()
            }
        }
        _ => {
            // Generic: show first string argument
            args.as_object()
                .and_then(|obj| obj.values().find_map(|v| v.as_str().map(|s| s.to_string())))
                .unwrap_or_default()
        }
    }
}
