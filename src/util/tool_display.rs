use std::collections::HashMap;

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
