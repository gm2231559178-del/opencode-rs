use crate::config::Config;
use crate::llm::provider::*;
use crate::llm::{create_provider, default_model};
use crate::tools::{builtin_tools, Tool, ToolContext, ToolResult};
use anyhow::Result;
use futures::StreamExt;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const DEFAULT_SYSTEM_PROMPT: &str = r"You are OpenCode, an AI coding agent. You help users with software engineering tasks.

You have access to tools that let you read, write, and execute commands in the user's environment. When you need to perform an action, respond with a tool call using the available functions. You can call multiple tools in sequence.

Available tools:
- read: Read file or directory contents
- write: Write content to a file (creates or overwrites)
- edit: Replace exact text in an existing file
- bash: Execute shell commands
- grep: Search file contents with regex
- glob: Find files matching a pattern
- task: Delegate work to a sub-agent for complex tasks

Rules:
1. Always use tools when you need to read or modify files.
2. When you run bash commands, prefer non-interactive commands.
3. If a task requires multiple steps, use tools step by step.
4. Explain your plan clearly before executing actions.";

pub struct UndoEntry {
    pub file_path: String,
    pub original_content: Option<String>,
}

pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    pub config: Arc<Config>,
    pub provider: Box<dyn LLMProvider>,
    pub tools: Vec<Box<dyn Tool>>,
    pub model: String,
    pub system_prompt: String,
    pub cwd: String,
    pub last_response: String,
    pub snapshots: Vec<UndoEntry>,
    pub plan_mode: bool,
}

impl Session {
    pub fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);
        let provider = create_provider(&config)?;

        let model = config
            .model
            .clone()
            .unwrap_or_else(|| "openai/gpt-4o".to_string());
        let provider_name = model.split('/').next().unwrap_or("openai");

        let model_id = model
            .splitn(2, '/')
            .nth(1)
            .unwrap_or_else(|| default_model(provider_name));

        let system_prompt = config
            .instructions
            .as_ref()
            .map(|i| i.join("\n"))
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

        let tools = builtin_tools();
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string());

        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            messages: Vec::new(),
            config,
            provider,
            tools,
            model: model_id.to_string(),
            last_response: String::new(),
            snapshots: Vec::new(),
            system_prompt,
            cwd,
            plan_mode: false,
        })
    }

    pub async fn prompt(&mut self, input: &str) -> Result<()> {
        info!(input_len = %input.len(), "prompt (non-streaming): start");
        debug!(input = %input, "prompt (non-streaming)");

        self.validate_messages();

        self.messages.push(Message {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: input.to_string(),
            }],
        });

        self.loop_().await
    }

    async fn loop_(&mut self) -> Result<()> {
        let max_iterations = 50;

        for _ in 0..max_iterations {
            let tool_defs: Vec<ToolDef> = self
                .tools
                .iter()
                .map(|t| ToolDef {
                    name: t.name().to_string(),
                    description: t.description().to_string(),
                    input_schema: t.parameters(),
                })
                .collect();

            let request = LLMRequest {
                model: self.model.clone(),
                messages: self.messages.clone(),
                tools: tool_defs,
                system: Some(self.system_prompt.clone()),
                max_tokens: Some(4096),
                temperature: None,
            };

            let msg_count = self.messages.len();
            info!(
                model = %self.model,
                msg_count = %msg_count,
                "loop_: sending LLM request"
            );
            let response = self.provider.generate(&request).await?;
            info!(
                content_len = %response.content.len(),
                tool_call_count = %response.tool_calls.len(),
                finish_reason = ?response.finish_reason,
                "loop_: LLM response"
            );

            let mut parts: Vec<ContentPart> = Vec::new();

            if !response.content.is_empty() {
                self.last_response.push_str(&response.content);
                self.last_response.push('\n');
                parts.push(ContentPart::Text {
                    text: response.content.clone(),
                });
            }

            for tc in &response.tool_calls {
                parts.push(ContentPart::ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                });
            }

            self.messages.push(Message {
                role: Role::Assistant,
                content: parts,
            });

            if response.finish_reason == FinishReason::Stop {
                break;
            }

            if response.finish_reason == FinishReason::ToolCalls {
                for tc in &response.tool_calls {
                    let result = self.execute_tool(tc).await;
                    let msg = Message {
                        role: Role::Tool,
                        content: vec![ContentPart::ToolResult {
                            tool_call_id: tc.id.clone(),
                            content: result.output.clone(),
                        }],
                    };
                    self.last_response
                        .push_str(&format!("  [Tool: {} - {}]\n", tc.name, result.title));
                    self.messages.push(msg);
                }
                continue;
            }

            break;
        }

        Ok(())
    }

    fn tool_permission(&self, name: &str) -> PermissionAction {
        if self.plan_mode {
            match name {
                "bash" | "write" | "edit" => return PermissionAction::Deny,
                _ => return PermissionAction::Allow,
            }
        }
        match name {
            "bash" | "write" | "edit" => PermissionAction::Ask,
            _ => PermissionAction::Allow,
        }
    }

    pub async fn prompt_stream(
        &mut self,
        input: &str,
        tx: mpsc::Sender<StreamEvent>,
        cancelled: Arc<AtomicBool>,
        perm_rx: &mut mpsc::UnboundedReceiver<(String, PermissionAction)>,
    ) {
        info!(input_len = input.len(), "prompt_stream: start");
        debug!(input = %input, "prompt_stream: user input");

        self.validate_messages();

        let msg_count = self.messages.len();
        self.messages.push(Message {
            role: Role::User,
            content: vec![ContentPart::Text {
                text: input.to_string(),
            }],
        });

        // Pop the user message on early exit (API error, cancellation before assistant reply)
        macro_rules! cleanup_user_msg {
            () => {
                self.messages.truncate(msg_count);
            };
        }

        let max_iterations = 50;
        for iter in 0..max_iterations {
            if cancelled.load(Ordering::SeqCst) {
                info!(iteration = %iter, "prompt_stream: cancelled");
                let _ = tx
                    .send(StreamEvent::Done {
                        response: self.last_response.clone(),
                    })
                    .await;
                cleanup_user_msg!();
                return;
            }
            let tool_defs: Vec<ToolDef> = self
                .tools
                .iter()
                .map(|t| ToolDef {
                    name: t.name().to_string(),
                    description: t.description().to_string(),
                    input_schema: t.parameters(),
                })
                .collect();

            let request = LLMRequest {
                model: self.model.clone(),
                messages: self.messages.clone(),
                tools: tool_defs,
                system: Some(self.system_prompt.clone()),
                max_tokens: Some(4096),
                temperature: None,
            };

            info!(
                iteration = %iter, model = %self.model,
                msg_count = %self.messages.len(),
                "prompt_stream: sending LLM request"
            );
            let mut stream = match self.provider.stream(&request).await {
                Ok(s) => {
                    info!(iteration = %iter, "prompt_stream: stream started");
                    s
                }
                Err(e) => {
                    error!(iteration = %iter, error = %e, "prompt_stream: LLM stream failed");
                    let _ = tx
                        .send(StreamEvent::Error {
                            message: format!("{}", e),
                        })
                        .await;
                    cleanup_user_msg!();
                    return;
                }
            };
            let mut response_text = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut finish_reason = FinishReason::Stop;

            while let Some(event) = stream.next().await {
                match event {
                    LLMEvent::Text { delta } => {
                        response_text.push_str(&delta);
                        let _ = tx.send(StreamEvent::Text { delta }).await;
                    }
                    LLMEvent::ToolCallStart { id, name } => {
                        debug!(call_id = %id, tool = %name, "prompt_stream: tool call start");
                        tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments: Value::Null,
                        });
                    }
                    LLMEvent::ToolCallDelta { id, delta } => {
                        if let Some(tc) = tool_calls.iter_mut().find(|tc| tc.id == id) {
                            let current = tc.arguments.as_str().unwrap_or("").to_string();
                            let merged = current + &delta;
                            tc.arguments =
                                serde_json::from_str(&merged).unwrap_or(Value::String(merged));
                        }
                    }
                    LLMEvent::Finish {
                        finish_reason: fr, ..
                    } => {
                        info!(
                            reason = ?fr,
                            response_len = %response_text.len(),
                            tool_call_count = %tool_calls.len(),
                            "prompt_stream: LLM finish"
                        );
                        finish_reason = fr;
                    }
                    LLMEvent::Error { message } => {
                        error!(message = %message, "prompt_stream: LLM event error");
                        let _ = tx.send(StreamEvent::Error { message }).await;
                        cleanup_user_msg!();
                        return;
                    }
                }
            }

            if !response_text.is_empty() {
                self.last_response.push_str(&response_text);
                self.last_response.push('\n');
            }

            let mut parts: Vec<ContentPart> = Vec::new();
            if !response_text.is_empty() {
                parts.push(ContentPart::Text {
                    text: response_text.clone(),
                });
            }
            for tc in &tool_calls {
                parts.push(ContentPart::ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                });
            }

            self.messages.push(Message {
                role: Role::Assistant,
                content: parts,
            });

            if tool_calls.is_empty() {
                info!(response_len = %response_text.len(), "prompt_stream: finish (no tool calls)");
                let _ = tx
                    .send(StreamEvent::Done {
                        response: response_text,
                    })
                    .await;
                break;
            }

            info!(
                tool_count = %tool_calls.len(),
                finish_reason = ?finish_reason,
                "prompt_stream: tool calls requested"
            );
            for tc in &tool_calls {
                    if cancelled.load(Ordering::SeqCst) {
                        info!("prompt_stream: cancelled during tool calls");
                        cleanup_user_msg!();
                        let _ = tx
                            .send(StreamEvent::Done {
                                response: self.last_response.clone(),
                            })
                            .await;
                        return;
                    }

                    debug!(tool = %tc.name, "prompt_stream: sending tool call event");
                    let _ = tx
                        .send(StreamEvent::ToolCall {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        })
                        .await;

                    let action = self.tool_permission(&tc.name);
                    info!(tool = %tc.name, action = ?action, "prompt_stream: permission check");
                    let allowed = match action {
                        PermissionAction::Allow => true,
                        PermissionAction::Deny => false,
                        PermissionAction::Ask => {
                            let request_id = uuid::Uuid::new_v4().to_string();
                            let _ = tx
                                .send(StreamEvent::PermissionRequest {
                                    request_id: request_id.clone(),
                                    tool_name: tc.name.clone(),
                                    args: tc.arguments.clone(),
                                })
                                .await;

                            loop {
                                match perm_rx.recv().await {
                                    Some((id, resp)) if id == request_id => {
                                        let granted = resp == PermissionAction::Allow;
                                        info!(tool = %tc.name, granted = %granted, "permission response");
                                        break granted;
                                    }
                                    Some(_) => continue,
                                    None => {
                                        info!("permission channel closed, aborting");
                                        cleanup_user_msg!();
                                        let _ = tx
                                            .send(StreamEvent::Done {
                                                response: self.last_response.clone(),
                                            })
                                            .await;
                                        return;
                                    }
                                }
                            }
                        }
                    };

                    if allowed {
                        self.snapshot_before_tool(tc);
                    }

                    let result = if allowed {
                        info!(tool = %tc.name, "prompt_stream: executing tool");
                        let r = self.execute_tool(tc).await;
                        info!(tool = %tc.name, title = %r.title, output_len = %r.output.len(), "tool execution complete");
                        r
                    } else {
                        info!(tool = %tc.name, "tool denied by user");
                        ToolResult {
                            title: format!("{} denied by user", tc.name),
                            output: format!("Permission denied for {}", tc.name),
                            metadata: Value::Null,
                        }
                    };

                    let msg = Message {
                        role: Role::Tool,
                        content: vec![ContentPart::ToolResult {
                            tool_call_id: tc.id.clone(),
                            content: result.output.clone(),
                        }],
                    };
                    self.last_response
                        .push_str(&format!("  [Tool: {} - {}]\n", tc.name, result.title));
                    self.messages.push(msg);

                    let _ = tx
                        .send(StreamEvent::ToolResult {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            output: result.output.clone(),
                        })
                        .await;
                }

            continue;

            break;
        }
    }

    fn snapshot_before_tool(&mut self, tc: &ToolCall) {
        let file_path = match tc.name.as_str() {
            "edit" | "write" => tc.arguments["file_path"].as_str().map(|s| s.to_string()),
            _ => None,
        };
        if let Some(path) = file_path {
            let content = std::fs::read_to_string(&path).ok();
            self.snapshots.push(UndoEntry {
                file_path: path,
                original_content: content,
            });
        }
    }

    pub fn undo_last(&mut self) -> String {
        if let Some(entry) = self.snapshots.pop() {
            match entry.original_content {
                Some(content) => match std::fs::write(&entry.file_path, &content) {
                    Ok(()) => format!("Undone: restored {}", entry.file_path),
                    Err(e) => format!("Undo failed for {}: {}", entry.file_path, e),
                },
                None => match std::fs::remove_file(&entry.file_path) {
                    Ok(()) => format!("Undone: removed new file {}", entry.file_path),
                    Err(e) => format!("Undo failed for {}: {}", entry.file_path, e),
                },
            }
        } else {
            "Nothing to undo.".to_string()
        }
    }

    pub fn show_diff(&self) -> String {
        let entry = match self.snapshots.last() {
            Some(e) => e,
            None => return "No edits to diff.".to_string(),
        };
        let old = match &entry.original_content {
            Some(c) => c.clone(),
            None => return format!("Diff for new file: {}", entry.file_path),
        };
        let new = match std::fs::read_to_string(&entry.file_path) {
            Ok(c) => c,
            Err(e) => return format!("Cannot read file: {}", e),
        };
        if old == new {
            return format!("No changes to {}", entry.file_path);
        }
        let mut out = format!("--- a/{}\n+++ b/{}\n", entry.file_path, entry.file_path);
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();
        let mut i = 0;
        let mut j = 0;

        // Simple Myers-like diff: walk lines, output +/- prefix
        while i < old_lines.len() || j < new_lines.len() {
            if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
                out.push_str(&format!(" {}\n", old_lines[i]));
                i += 1;
                j += 1;
            } else if i < old_lines.len() && j < new_lines.len() {
                out.push_str(&format!("-{}\n+{}\n", old_lines[i], new_lines[j]));
                i += 1;
                j += 1;
            } else if i < old_lines.len() {
                out.push_str(&format!("-{}\n", old_lines[i]));
                i += 1;
            } else {
                out.push_str(&format!("+{}\n", new_lines[j]));
                j += 1;
            }
        }
        out
    }

    pub fn compact_messages(&mut self) -> usize {
        let before = self.messages.len();
        // Find the last assistant+tools block and keep everything from there.
        // Drop older tool results that are no longer needed.
        let mut last_assistant_with_tools = None;
        for (i, msg) in self.messages.iter().enumerate().rev() {
            if msg.role == Role::Assistant {
                let has_tc = msg.content.iter().any(|p| matches!(p, ContentPart::ToolCall { .. }));
                if has_tc {
                    last_assistant_with_tools = Some(i);
                    break;
                }
            }
        }
        if let Some(pivot) = last_assistant_with_tools {
            self.messages.drain(..pivot);
        }
        before - self.messages.len()
    }

    fn validate_messages(&mut self) {
        // Remove incomplete tool-call turns that would cause 400 errors.
        // An assistant(tool_calls) must be followed by tool results for each call_id.
        // Scan from the end so only the most recent incomplete turn is removed.
        let mut i = self.messages.len();
        while i > 1 {
            i -= 1;
            if self.messages[i].role == Role::Assistant {
                let has_tool_calls = self.messages[i].content.iter().any(|p| matches!(p, ContentPart::ToolCall { .. }));
                if !has_tool_calls {
                    break;
                }
                let n_tool_calls = self.messages[i].content.iter()
                    .filter(|p| matches!(p, ContentPart::ToolCall { .. }))
                    .count();
                let n_results: usize = self.messages[(i+1)..].iter()
                    .filter(|m| m.role == Role::Tool)
                    .map(|m| m.content.len())
                    .sum();
                if n_results < n_tool_calls {
                    if i > 0 && self.messages[i-1].role == Role::User {
                        self.messages.truncate(i - 1);
                    } else {
                        self.messages.truncate(i);
                    }
                    continue;
                }
                break;
            }
        }
    }

    async fn execute_tool(&self, tc: &ToolCall) -> ToolResult {
        let tool = self.tools.iter().find(|t| t.name() == tc.name);

        match tool {
            Some(t) => {
                let args_preview: String = serde_json::to_string(&tc.arguments)
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect();
                debug!(tool = %tc.name, args = %args_preview, "execute_tool: running");

                let ctx = ToolContext {
                    session_id: self.id.clone(),
                    message_id: String::new(),
                    cwd: self.cwd.clone(),
                    config: Some(self.config.clone()),
                };
                let result = t.execute(tc.arguments.clone(), &ctx).await;
                match &result {
                    Ok(r) => {
                        debug!(
                            tool = %tc.name,
                            title = %r.title,
                            output_len = %r.output.len(),
                            "execute_tool: success"
                        );
                        r.clone()
                    }
                    Err(e) => {
                        warn!(tool = %tc.name, error = %e, "execute_tool: error");
                        ToolResult {
                            title: format!("Error: {}", e),
                            output: format!("Error executing {}: {}", tc.name, e),
                            metadata: Value::Null,
                        }
                    }
                }
            }
            None => {
                warn!(tool = %tc.name, "execute_tool: tool not found");
                ToolResult {
                    title: format!("Unknown tool: {}", tc.name),
                    output: format!("Tool '{}' not found", tc.name),
                    metadata: Value::Null,
                }
            }
        }
    }
}
