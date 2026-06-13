use crate::llm::provider::{PermissionAction, StreamEvent};
use crate::session::Session;
use crate::session_store::SessionStore;
use crate::theme::Theme;
use anyhow::Result;
use arboard::Clipboard;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

pub struct TuiApp {
    pub session: Arc<Mutex<Session>>,
    pub messages: Vec<TuiMessage>,
    pub input: String,
    pub cursor: usize,
    pub input_history: Vec<String>,
    pub history_index: isize,
    pub saved_input: String,
    pub scroll: usize,
    pub quit: bool,
    pub stream_rx: Option<mpsc::Receiver<StreamEvent>>,
    pub pending_response: String,
    pub streaming: bool,
    pub cancelled: Arc<AtomicBool>,
    pub model_name: String,
    pub prompt_count: usize,
    pub perm_tx: mpsc::UnboundedSender<(String, PermissionAction)>,
    pub pending_perm: Option<String>,
    pub store: Option<SessionStore>,
    pub plan_mode: bool,
    pub autocomplete_candidates: Vec<String>,
    pub autocomplete_idx: isize,
    pub theme: &'static Theme,
    pub theme_name: String,
    pub notify: bool,
    pub reasoning: String,
    pub reasoning_visible: bool,
    pub collapsed: std::collections::HashSet<usize>,
    pub toast: Option<(String, u8)>,
    pub leader_mode: bool,
    pub file_watcher_rx: Option<std_mpsc::Receiver<String>>,
    pub diff_viewer: Option<(Vec<String>, usize)>,  // (lines, scroll_offset)
}

#[derive(Clone)]
pub struct TuiMessage {
    pub role: String,
    pub content: String,
    pub age: u8,
}

const SLASH_COMMANDS: &[&str] = &[
    "/help", "/plan", "/compact", "/diff", "/theme", "/theme <name>",
    "/notify", "/new", "/model", "/model <name>", "/agent", "/agent <name>",
    "/agents", "/version", "/sessions", "/session load <id>", "/session fork",
    "/session rename <id> <name>", "/session delete <id>", "/session new",
    "/undo", "/share", "/share list", "/share import <id> <secret>",
    "/stats", "/mcp", "/plugin", "/diagnostics <file>", "/exit",
];

impl TuiApp {
    pub fn new(session: Session, store: Option<SessionStore>) -> Self {
        let model_name = session.model.clone();
        Self {
            session: Arc::new(Mutex::new(session)),
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            input_history: Vec::new(),
            history_index: -1,
            saved_input: String::new(),
            scroll: 0,
            quit: false,
            stream_rx: None,
            pending_response: String::new(),
            streaming: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            model_name,
            prompt_count: 0,
            perm_tx: mpsc::unbounded_channel().0,
            pending_perm: None,
            store,
            plan_mode: false,
            autocomplete_candidates: Vec::new(),
            autocomplete_idx: -1,
            theme: &crate::theme::DEFAULT,
            theme_name: "default".to_string(),
            notify: true,
            reasoning: String::new(),
            reasoning_visible: true,
            collapsed: std::collections::HashSet::new(),
            toast: None,
            leader_mode: false,
            file_watcher_rx: None,
            diff_viewer: None,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;

        let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;

        // Start file watcher for the current directory
        if let Ok(cwd) = std::env::current_dir() {
            self.start_file_watcher(&cwd.to_string_lossy());
        }

        let result = self.run_loop(&mut terminal).await;

        disable_raw_mode()?;
        terminal.backend_mut().execute(LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        result
    }

    async fn run_loop(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        while !self.quit {
            self.poll_stream();
            self.poll_file_watcher();

            terminal.draw(|f| self.render(f))?;

            if crossterm::event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key).await?;
                    }
                }
            } else {
                tokio::task::yield_now().await;
            }
        }
        Ok(())
    }

    fn start_file_watcher(&mut self, watch_dir: &str) {
        use notify::{Config, Event, EventKind, RecursiveMode, Watcher};
        let (tx, rx) = std_mpsc::channel();
        let dir = watch_dir.to_string();
        std::thread::spawn(move || {
            let mut watcher = match notify::RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        let paths: Vec<String> = event
                            .paths
                            .iter()
                            .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()))
                            .collect();
                        if !paths.is_empty() {
                            let msg = format!("File changed: {}", paths.join(", "));
                            let _ = tx.send(msg);
                        }
                    }
                },
                Config::default(),
            ) {
                Ok(w) => w,
                Err(_) => return,
            };
            if let Err(e) = watcher.watch(Path::new(&dir), RecursiveMode::Recursive) {
                eprintln!("File watcher error: {}", e);
                return;
            }
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        self.file_watcher_rx = Some(rx);
    }

    fn poll_file_watcher(&mut self) {
        while let Some(rx) = &self.file_watcher_rx {
            match rx.try_recv() {
                Ok(msg) => {
                    self.show_toast(msg);
                }
                Err(std_mpsc::TryRecvError::Empty) => break,
                Err(std_mpsc::TryRecvError::Disconnected) => {
                    self.file_watcher_rx = None;
                    break;
                }
            }
        }
    }

    fn poll_stream(&mut self) {
        // Age all messages for fade-in animation
        for m in &mut self.messages {
            m.age = m.age.saturating_add(1);
        }

        if !self.streaming {
            return;
        }
        let mut done = false;
        if let Some(rx) = &mut self.stream_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    StreamEvent::Reasoning { delta } => {
                        self.reasoning.push_str(&delta);
                    }
                    StreamEvent::Text { delta } => {
                        let needs_new = self.messages.last().map(|m| m.role != "assistant").unwrap_or(true);
                        if needs_new {
                            self.messages.push(TuiMessage {
                                age: 0,
                                role: "assistant".to_string(),
                                content: delta,
                            });
                        } else if let Some(last) = self.messages.last_mut() {
                            last.content.push_str(&delta);
                        }
                    }
                    StreamEvent::ToolCall { id, name, arguments } => {
                        if let Some(last) = self.messages.last() {
                            if last.role == "assistant" && last.content.is_empty() {
                                self.messages.pop();
                            }
                        }
                        let short = if id.len() > 8 { &id[..8] } else { &id };
                        let args_str = serde_json::to_string_pretty(&arguments)
                            .unwrap_or_default();
                        let preview: String = args_str.chars().take(400).collect();
                        self.messages.push(TuiMessage {
            age: 0,
                            role: "tool_call".to_string(),
                            content: format!("{} ({})\n{}", name, short, preview),
                        });
                    }
                    StreamEvent::PermissionRequest { request_id, tool_name, args } => {
                        let args_str = serde_json::to_string_pretty(&args).unwrap_or_default();
                        let preview: String = args_str.chars().take(200).collect();
                        self.messages.push(TuiMessage {
            age: 0,
                            role: "tool_call".to_string(),
                            content: format!("{} (AWAITING APPROVAL)\n{}", tool_name, preview),
                        });
                        self.pending_perm = Some(request_id);
                    }
                    StreamEvent::ToolResult { name, output, .. } => {
                        let lines: Vec<&str> = output.lines().collect();
                        let max_lines = 100;
                        let max_chars = 2000;
                        let truncated_lines = lines.len() > max_lines;
                        let shown_lines: Vec<&str> = lines.into_iter().take(max_lines).collect();
                        let shown = shown_lines.join("\n");
                        let truncated_chars = shown.len() > max_chars;
                        let preview: String = shown.chars().take(max_chars).collect();
                        let content = if truncated_lines {
                            format!("{} ({} lines, showing first {})\n{}", name, output.len(), max_lines, preview)
                        } else if truncated_chars {
                            format!("{} ({} chars, showing first {})\n{}", name, output.len(), max_chars, preview)
                        } else {
                            format!("{} ({} chars)\n{}", name, output.len(), preview)
                        };
                        self.messages.push(TuiMessage {
            age: 0,
                            role: "tool_result".to_string(),
                            content,
                        });
                    }
                    StreamEvent::Done { response } => {
                        if self.notify {
                            let _ = print!("\x07");
                            let _ = io::Write::flush(&mut io::stdout());
                        }

                        if !self.reasoning.is_empty() {
                            let reasoning = std::mem::take(&mut self.reasoning);
                            self.messages.push(TuiMessage {
                                age: 0,
                                role: "reasoning".to_string(),
                                content: reasoning,
                            });
                        }

                        if response.is_empty() {
                            if let Some(last) = self.messages.last() {
                                if last.role == "assistant" {
                                    self.messages.pop();
                                }
                            }
                        } else {
                            let updated = self.messages.iter_mut().rev().find(|m| m.role == "assistant");
                            if let Some(msg) = updated {
                                msg.content = response.trim().to_string();
                            } else {
                                self.messages.push(TuiMessage {
                                    age: 0,
                                    role: "assistant".to_string(),
                                    content: response.trim().to_string(),
                                });
                            }
                        }
                        self.pending_response.clear();
                        done = true;
                    }
                    StreamEvent::Error { message } => {
                        if self.notify {
                            let _ = print!("\x07");
                            let _ = io::Write::flush(&mut io::stdout());
                        }
                        let updated = self.messages.iter_mut().rev().find(|m| m.role == "assistant");
                        if let Some(msg) = updated {
                            msg.content = format!("Error: {}", message);
                        } else {
                            self.messages.push(TuiMessage {
            age: 0,
                                role: "assistant".to_string(),
                                content: format!("Error: {}", message),
                            });
                        }
                        self.pending_response.clear();
                        done = true;
                    }
                }
            }
        }
        if done {
            self.streaming = false;
            self.stream_rx = None;
            self.save_session();
        }
    }

    fn save_session(&self) {
        if let Some(store) = &self.store {
            if let Ok(session) = self.session.try_lock() {
                let _ = store.save_session(
                    &session.id,
                    &session.model,
                    &session.system_prompt,
                    &session.cwd,
                    &session.messages,
                );
            }
        }
    }

    async fn handle_slash(&mut self, cmd: &str) {
        let response = match cmd {
            "/sessions" => self.cmd_list_sessions(10),
            "/session load" | "/session continue" => {
                "Usage: /session load <session_id>".to_string()
            }
            cmd if cmd.starts_with("/session load ") || cmd.starts_with("/session continue ") => {
                let id = cmd.splitn(3, ' ').nth(2).unwrap_or("").trim();
                self.cmd_load_session(id)
            }
            cmd if cmd.starts_with("/session delete ") => {
                let id = cmd.splitn(3, ' ').nth(2).unwrap_or("").trim();
                self.cmd_delete_session(id)
            }
            "/session fork" => self.cmd_fork_session(),
            cmd if cmd.starts_with("/session rename ") => {
                let parts: Vec<&str> = cmd.splitn(4, ' ').collect();
                if parts.len() < 4 {
                    "Usage: /session rename <id> <new_name>".to_string()
                } else if let Some(store) = &self.store {
                    match store.rename_session(parts[2], parts[3]) {
                        Ok(()) => format!("Session {} renamed to '{}'", &parts[2][..8], parts[3]),
                        Err(e) => format!("Rename failed: {}", e),
                    }
                } else {
                    "Session store not available.".to_string()
                }
            }
            "/session new" => {
                self.cmd_clear_session();
                if let Ok(mut s) = self.session.try_lock() {
                    s.id = uuid::Uuid::new_v4().to_string();
                }
                "New session created.".to_string()
            }
            "/plan" => {
                self.plan_mode = !self.plan_mode;
                if let Ok(mut s) = self.session.try_lock() {
                    s.plan_mode = self.plan_mode;
                    if self.plan_mode {
                        let plan_instructions = "You are in PLAN MODE. Do NOT execute any commands or make any edits. Your job is only to read files, explore the codebase, and produce a detailed plan. Do not use bash, write, or edit tools.";
                        s.system_prompt = format!("{}\n\n{}", s.system_prompt, plan_instructions);
                    }
                }
                if self.plan_mode {
                    "Plan mode ON — only read tools allowed. Tab to toggle.".to_string()
                } else {
                    "Plan mode OFF.".to_string()
                }
            }
            "/undo" => {
                match self.session.try_lock() {
                    Ok(mut s) => s.undo_last(),
                    Err(_) => "Session busy, try again.".to_string(),
                }
            }
            "/compact" => {
                match self.session.try_lock() {
                    Ok(mut s) => {
                        let removed = s.compact_messages();
                        format!("Compacted: removed {} old messages.", removed)
                    }
                    Err(_) => "Session busy, try again.".to_string(),
                }
            }
            "/share" => {
                if let Some(store) = &self.store {
                    if let Ok(session) = self.session.try_lock() {
                        match store.share_session(&session.id) {
                            Ok(info) => format!("Session shared!\nID + Secret:\n{}", info),
                            Err(e) => format!("Share failed: {}", e),
                        }
                    } else {
                        "Session busy.".to_string()
                    }
                } else {
                    "Session store not available.".to_string()
                }
            }
            cmd if cmd.starts_with("/share import ") => {
                let parts: Vec<&str> = cmd.splitn(4, ' ').collect();
                if parts.len() < 4 {
                    "Usage: /share import <share_id> <secret>".to_string()
                } else if let Some(store) = &self.store {
                    match store.import_shared_session(parts[2], parts[3]) {
                        Ok(session_id) => format!("Imported as session: {}", &session_id[..8]),
                        Err(e) => format!("Import failed: {}", e),
                    }
                } else {
                    "Session store not available.".to_string()
                }
            }
            "/share list" => {
                if let Some(store) = &self.store {
                    match store.list_shares() {
                        Ok(shares) if shares.is_empty() => "No shared sessions.".to_string(),
                        Ok(shares) => {
                            let mut out = "Shared sessions:\n".to_string();
                            for s in &shares {
                                out.push_str(&format!(
                                    "  {} | {} | {}\n",
                                    &s.id[..8], s.model, s.created_at
                                ));
                            }
                            out
                        }
                        Err(e) => format!("Error: {}", e),
                    }
                } else {
                    "Session store not available.".to_string()
                }
            }
            "/stats" => {
                if let Ok(session) = self.session.try_lock() {
                    let s = &session.stats;
                    format!(
                        "Usage stats:\n  Prompts:       {}\n  Tool calls:    {}\n  Prompt tokens: {}\n  Completion tk: {}\n  Total tokens:  {}",
                        s.prompt_count, s.tool_call_count, s.prompt_tokens, s.completion_tokens, s.total_tokens
                    )
                } else {
                    "Session busy.".to_string()
                }
            }
            "/mcp" => {
                if let Ok(session) = self.session.try_lock() {
                    let tool_count = session.tools.len();
                    let mcp_count = session.tools.iter().filter(|t| t.name().starts_with("mcp_")).count();
                    format!("MCP servers connected.\nTotal tools: {} ({} MCP)", tool_count, mcp_count)
                } else {
                    "Session busy.".to_string()
                }
            }
            "/plugin" => {
                if let Ok(session) = self.session.try_lock() {
                    let plugin_count = session.tools.iter().filter(|t| t.name().starts_with("plugin_")).count();
                    format!("Plugins loaded: {} tools from config-based plugins.", plugin_count)
                } else {
                    "Session busy.".to_string()
                }
            }
            "/diagnostics" | "/diag" => {
                "Usage: /diagnostics <file_path>".to_string()
            }
            cmd if cmd.starts_with("/diagnostics ") || cmd.starts_with("/diag ") => {
                let file_path = cmd.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                let cwd = self.session.try_lock().map(|s| s.cwd.clone()).unwrap_or_default();
                let full_path = if file_path.starts_with('/') {
                    file_path
                } else {
                    format!("{}/{}", cwd, file_path)
                };
                match crate::lsp::LspManager::new().open_file(&full_path).await {
                    Ok(diags) if diags.is_empty() => "No diagnostics found.".to_string(),
                    Ok(diags) => {
                        let mut out = format!("Diagnostics for {}:\n", full_path);
                        for d in &diags {
                            let sev = match d.severity {
                                Some(s) if s == lsp_types::DiagnosticSeverity::ERROR => "error",
                                Some(s) if s == lsp_types::DiagnosticSeverity::WARNING => "warning",
                                _ => "info",
                            };
                            let range = &d.range;
                            out.push_str(&format!(
                                "  {}:{}:{} {}: {}\n",
                                sev,
                                range.start.line + 1,
                                range.start.character + 1,
                                d.source.as_deref().unwrap_or("lsp"),
                                d.message,
                            ));
                        }
                        out
                    }
                    Err(e) => format!("LSP error: {}", e),
                }
            }
            "/diff" => {
                match self.session.try_lock() {
                    Ok(s) => {
                        let diff = s.show_diff();
                        if diff.starts_with("---") {
                            let lines: Vec<String> = diff.lines().map(|l| l.to_string()).collect();
                            self.diff_viewer = Some((lines, 0));
                            String::new()
                        } else {
                            diff
                        }
                    }
                    Err(_) => "Session busy, try again.".to_string(),
                }
            }
            "/notify" => {
                self.notify = !self.notify;
                if self.notify {
                    "Notifications ON.".to_string()
                } else {
                    "Notifications OFF.".to_string()
                }
            }
            "/theme" => {
                let names = ["default", "tokyonight", "catppuccin", "gruvbox", "dracula", "nord", "onedark"];
                format!("Current theme: {}\nAvailable: {}", self.theme_name, names.join(", "))
            }
            cmd if cmd.starts_with("/theme ") => {
                let name = cmd.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                self.theme = Theme::by_name(&name);
                self.theme_name = self.theme.name.to_string();
                format!("Switched to theme: {}", self.theme.name)
            }
            "/help" => "Available commands:\n  /help           - Show this help\n  /plan           - Toggle plan mode (read-only)\n  /compact        - Compact conversation history\n  /diff           - Show diff of last file edit\n  /theme          - Show current theme\n  /theme <name>   - Switch theme\n  /notify         - Toggle notification bell\n  /new            - Clear session\n  /model          - Show current model\n  /model <name>   - Switch model (e.g. /model openai/gpt-4o)\n  /agent          - Show available agents\n  /agent <name>   - Switch agent\n  /agents         - Show AGENTS.md workspace instructions\n  /version        - Show version info\n  /sessions       - List saved sessions\n  /session load <id>   - Load a saved session\n  /session fork        - Fork current session\n  /session rename <id> <name> - Rename a session\n  /session delete <id> - Delete a session\n  /undo           - Undo last file change\n  /share          - Generate share link for this session\n  /share list     - List shared sessions\n  /share import <id> <secret> - Import a shared session\n  /stats          - Show usage statistics\n  /mcp            - Show MCP server connection status\n  /plugin         - Show plugin status\n  /diagnostics <file> - Run LSP diagnostics on a file\n  /exit           - Quit OpenCode".to_string(),
            "/new" | "/clear" => self.cmd_clear_session(),
            "/models" => self.cmd_show_model(),
            "/model" => self.cmd_show_model(),
            cmd if cmd.starts_with("/model ") => {
                let name = cmd.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                self.cmd_set_model(name)
            }
            "/agent" => self.cmd_list_agents(),
            cmd if cmd.starts_with("/agent ") => {
                let name = cmd.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                self.cmd_set_agent(name)
            }
            "/agents" => {
                let cwd = self.session.try_lock().map(|s| s.cwd.clone()).unwrap_or_default();
                let agents_path = std::path::Path::new(&cwd).join("AGENTS.md");
                let agents_path2 = std::path::Path::new(&cwd).join(".opencode").join("AGENTS.md");
                let content = if agents_path.exists() {
                    std::fs::read_to_string(&agents_path).unwrap_or_default()
                } else if agents_path2.exists() {
                    std::fs::read_to_string(&agents_path2).unwrap_or_default()
                } else {
                    String::new()
                };
                if content.trim().is_empty() {
                    "No AGENTS.md found in workspace.".to_string()
                } else {
                    format!("AGENTS.md:\n{}", content)
                }
            }
            "/version" => {
                format!("opencode-rs v{}", env!("CARGO_PKG_VERSION"))
            }
            "/exit" | "/quit" | "/q" => {
                self.quit = true;
                String::new()
            }
            _ => format!("Unknown command: {}\nType /help for available commands.", cmd),
        };
        if !response.is_empty() {
            self.messages.push(TuiMessage {
                age: 0,
                role: "assistant".to_string(),
                content: response,
            });
        }
    }

    fn cmd_list_sessions(&self, limit: usize) -> String {
        match &self.store {
            Some(store) => match store.list_sessions(limit) {
                Ok(sessions) if sessions.is_empty() => "No saved sessions.".to_string(),
                Ok(sessions) => {
                    let mut out = String::from("Recent sessions:\n");
                    for s in &sessions {
                        let preview = if s.id.len() > 8 { &s.id[..8] } else { &s.id };
                        out.push_str(&format!("  {} | {} | {} msgs | {}\n",
                            preview, s.model, s.message_count, s.updated_at));
                    }
                    out.push_str(&format!("\nUse /session load <id> to continue a session."));
                    out
                }
                Err(e) => format!("Error: {}", e),
            },
            None => "Session store not available.".to_string(),
        }
    }

    fn cmd_load_session(&mut self, id: &str) -> String {
        let store = match &self.store {
            Some(s) => s,
            None => return "Session store not available.".to_string(),
        };
        match store.load_session(id) {
            Ok(Some((_model, _system_prompt, _cwd, messages))) => {
                let n = messages.len();
                if let Ok(mut session) = self.session.try_lock() {
                    session.messages = messages;
                }
                self.messages.clear();
                self.prompt_count = 0;
                format!("Session {} loaded with {} messages.", id, n)
            }
            Ok(None) => format!("Session '{}' not found.", id),
            Err(e) => format!("Error loading session: {}", e),
        }
    }

    fn cmd_delete_session(&self, id: &str) -> String {
        let store = match &self.store {
            Some(s) => s,
            None => return "Session store not available.".to_string(),
        };
        match store.delete_session(id) {
            Ok(()) => format!("Session {} deleted.", id),
            Err(e) => format!("Error deleting session: {}", e),
        }
    }

    fn cmd_fork_session(&self) -> String {
        let new_id = uuid::Uuid::new_v4().to_string();
        if let Some(store) = &self.store {
            if let Ok(session) = self.session.try_lock() {
                let _ = store.save_session(
                    &new_id,
                    &session.model,
                    &session.system_prompt,
                    &session.cwd,
                    &session.messages,
                );
            }
        }
        format!("Forked as session {}", &new_id[..8])
    }

    fn cmd_clear_session(&mut self) -> String {
        self.messages.clear();
        self.prompt_count = 0;
        "Session cleared.".to_string()
    }

    fn cmd_show_model(&self) -> String {
        format!("Current model: {}\n\nAvailable providers: openai, anthropic, openrouter, groq, opencode\nSwitch with: /model <provider/model_id>", self.model_name)
    }

    fn cmd_set_model(&mut self, name: String) -> String {
        if name.is_empty() {
            return self.cmd_show_model();
        }
        if let Ok(mut session) = self.session.try_lock() {
            session.model = name.clone();
            self.model_name = name.clone();
            format!("Switched to model: {}", name)
        } else {
            "Session busy, try again.".to_string()
        }
    }

    fn cmd_list_agents(&self) -> String {
        if let Ok(session) = self.session.try_lock() {
            let mut out = String::from("Available agents:\n");
            for (name, _cfg) in &session.config.agent {
                out.push_str(&format!("  - {}\n", name));
            }
            if out == "Available agents:\n" {
                out.push_str("  (none configured)\n");
            }
            out.push_str("\nSwitch with: /agent <name>");
            out
        } else {
            "Session busy.".to_string()
        }
    }

    fn cmd_set_agent(&mut self, name: String) -> String {
        if name.is_empty() {
            return self.cmd_list_agents();
        }
        if let Ok(mut session) = self.session.try_lock() {
            let cfg = session.config.agent.get(&name).cloned();
            match cfg {
                Some(cfg) => {
                    if let Some(model) = &cfg.model {
                        session.model = model.clone();
                        self.model_name = model.clone();
                    }
                    if let Some(instructions) = &cfg.instructions {
                        session.system_prompt = instructions.join("\n");
                    }
                    format!("Switched to agent: {}", name)
                }
                None => format!("Agent '{}' not found. Use /agent to list available agents.", name),
            }
        } else {
            "Session busy, try again.".to_string()
        }
    }

    fn copy_last_response(&mut self) {
        let last = self.messages.iter().rev().find(|m| m.role == "assistant");
        match last {
            Some(msg) if !msg.content.is_empty() => {
                match Clipboard::new() {
                    Ok(mut ctx) => {
                        if ctx.set_text(msg.content.clone()).is_ok() {
                            self.messages.push(TuiMessage {
                                age: 0,
                                role: "assistant".to_string(),
                                content: "Last response copied to clipboard.".to_string(),
                            });
                        } else {
                            self.messages.push(TuiMessage {
                                age: 0,
                                role: "assistant".to_string(),
                                content: "Failed to copy to clipboard.".to_string(),
                            });
                        }
                    }
                    Err(_) => {
                        self.messages.push(TuiMessage {
                            age: 0,
                            role: "assistant".to_string(),
                            content: "Clipboard not available.".to_string(),
                        });
                    }
                }
            }
            _ => {
                self.messages.push(TuiMessage {
                    age: 0,
                    role: "assistant".to_string(),
                    content: "No response to copy.".to_string(),
                });
            }
        }
    }

    fn open_last_edited_file(&mut self) {
        let file_path = self
            .session
            .try_lock()
            .ok()
            .and_then(|s| s.snapshots.last().map(|e| e.file_path.clone()));
        match file_path {
            Some(path) if !path.is_empty() => {
                let editor = std::env::var("EDITOR")
                    .or_else(|_| std::env::var("VISUAL"))
                    .unwrap_or_else(|_| "vi".to_string());
                self.show_toast(format!("Opening {} in {}", &path, editor));
                std::thread::spawn(move || {
                    std::process::Command::new(&editor)
                        .arg(&path)
                        .spawn()
                        .ok();
                });
            }
            _ => {
                self.show_toast("No edited file to open".to_string());
            }
        }
    }

    fn trigger_autocomplete(&mut self) {
        let before_cursor = &self.input[..self.cursor];

        // Slash command autocomplete
        if before_cursor.starts_with('/') && !before_cursor.contains(' ') {
            let query = before_cursor.to_string();
            let mut candidates: Vec<String> = SLASH_COMMANDS
                .iter()
                .filter(|cmd| cmd.starts_with(&query))
                .map(|s| s.to_string())
                .collect();
            candidates.sort();
            if candidates.len() > 1 || (candidates.len() == 1 && candidates[0] != query) {
                self.autocomplete_candidates = candidates;
                self.autocomplete_idx = if self.autocomplete_candidates.is_empty() { -1 } else { 0 };
                return;
            }
        }

        // @ file autocomplete
        let at_pos = before_cursor.rfind('@');
        match at_pos {
            Some(pos) => {
                let query = before_cursor[pos + 1..].to_string();
                let file_query = query.split('#').next().unwrap_or(&query).to_string();
                let pattern = if file_query.is_empty() {
                    "*".to_string()
                } else {
                    format!("*{}*", file_query)
                };
                let mut cmd = std::process::Command::new("fd");
                cmd.arg("--glob").arg(&pattern).arg("--max-results").arg("20");
                if let Ok(session) = self.session.try_lock() {
                    cmd.current_dir(&session.cwd);
                }
                let output = cmd.output().ok();
                let mut candidates: Vec<String> = output
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.lines().map(|l| l.to_string()).collect())
                    .unwrap_or_default();
                if !file_query.is_empty() {
                    candidates.sort_by_key(|c| {
                        let lower_c = c.to_lowercase();
                        let lower_q = file_query.to_lowercase();
                        lower_c.find(&lower_q).unwrap_or(usize::MAX)
                    });
                }
                // Add line range info to candidates
                for c in &mut candidates {
                    let path = std::path::Path::new(c);
                    if path.is_dir() {
                        c.push('/');
                    }
                }
                self.autocomplete_candidates = candidates;
                self.autocomplete_idx = if self.autocomplete_candidates.is_empty() {
                    -1
                } else {
                    0
                };
            }
            None => {
                self.autocomplete_candidates.clear();
                self.autocomplete_idx = -1;
            }
        }
    }

    fn apply_autocomplete(&mut self) -> bool {
        if self.autocomplete_idx < 0 || self.autocomplete_idx as usize >= self.autocomplete_candidates.len() {
            return false;
        }
        let selected = &self.autocomplete_candidates[self.autocomplete_idx as usize];
        let before_cursor = &self.input[..self.cursor];

        // Slash command completion
        if before_cursor.starts_with('/') && !before_cursor.contains(' ') {
            let after_cursor = &self.input[self.cursor..];
            self.input = format!("{} {}", selected, after_cursor);
            self.cursor = selected.len() + 1;
            self.autocomplete_candidates.clear();
            self.autocomplete_idx = -1;
            return true;
        }

        if let Some(at_pos) = before_cursor.rfind('@') {
            let after_cursor = &self.input[self.cursor..];
            let after_at = &before_cursor[at_pos + 1..];
            let suffix = if let Some(hash_pos) = after_at.find('#') {
                after_at[hash_pos..].to_string()
            } else {
                String::new()
            };
            let replacement = if suffix.is_empty() {
                format!("{} ", selected)
            } else {
                format!("{}", selected)
            };
            let new_input = format!("{}{}{}", &self.input[..at_pos], replacement, after_cursor);
            let new_cursor = at_pos + replacement.len();
            self.input = new_input;
            self.cursor = new_cursor;
        }
        self.autocomplete_candidates.clear();
        self.autocomplete_idx = -1;
        true
    }

    async fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        // Diff viewer mode handles all keys when active
        if self.diff_viewer.is_some() {
            let lines_len = self.diff_viewer.as_ref().map(|v| v.0.len()).unwrap_or(0);
            let scroll = self.diff_viewer.as_ref().map(|v| v.1).unwrap_or(0);
            match key.code {
                KeyCode::Esc => { self.diff_viewer = None; }
                KeyCode::Up => {
                    let new_scroll = scroll.saturating_sub(1);
                    self.diff_viewer.as_mut().map(|v| v.1 = new_scroll);
                }
                KeyCode::Down => {
                    let new_scroll = (scroll + 1).min(lines_len.saturating_sub(1));
                    self.diff_viewer.as_mut().map(|v| v.1 = new_scroll);
                }
                KeyCode::PageUp => {
                    let new_scroll = scroll.saturating_sub(20);
                    self.diff_viewer.as_mut().map(|v| v.1 = new_scroll);
                }
                KeyCode::PageDown => {
                    let new_scroll = (scroll + 20).min(lines_len.saturating_sub(1));
                    self.diff_viewer.as_mut().map(|v| v.1 = new_scroll);
                }
                KeyCode::Home => {
                    self.diff_viewer.as_mut().map(|v| v.1 = 0);
                }
                KeyCode::End => {
                    self.diff_viewer.as_mut().map(|v| v.1 = lines_len.saturating_sub(1));
                }
                _ => {}
            }
            return Ok(());
        }

        // Leader mode: handle the action key
        if self.leader_mode {
            self.leader_mode = false;
            let action = match key.code {
                KeyCode::Char('f') => Some("/diagnostics "),
                KeyCode::Char('s') => Some("/sessions"),
                KeyCode::Char('/') => Some("/plan"),
                KeyCode::Char('t') => Some("/theme"),
                KeyCode::Char('n') => Some("/new"),
                KeyCode::Char('h') => Some("/help"),
                KeyCode::Char('d') => Some("/diff"),
                KeyCode::Char('e') => {
                    self.open_last_edited_file();
                    None
                }
                KeyCode::Char('q') => { self.quit = true; None }
                KeyCode::Char('m') => Some("/model "),
                KeyCode::Char('a') => Some("/agent "),
                KeyCode::Esc => None,
                _ => { self.show_toast("Unknown leader key".to_string()); None }
            };
            if let Some(cmd) = action {
                self.input = cmd.to_string();
                self.cursor = self.input.len();
                if !cmd.ends_with(' ') {
                    self.handle_slash(cmd).await;
                }
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
            }
            KeyCode::Char('q') if self.input.is_empty() => {
                self.quit = true;
            }
            KeyCode::Char(' ') if self.input.is_empty() && !self.streaming => {
                self.leader_mode = true;
            }
            KeyCode::Esc if self.streaming => {
                self.cancelled.store(true, Ordering::SeqCst);
            }
            KeyCode::Esc if !self.autocomplete_candidates.is_empty() => {
                self.autocomplete_candidates.clear();
                self.autocomplete_idx = -1;
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) && !self.streaming => {
                self.copy_last_response();
                self.show_toast("Copied last response to clipboard".to_string());
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) && !self.streaming => {
                self.open_last_edited_file();
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) && !self.streaming => {
                self.reasoning_visible = !self.reasoning_visible;
                self.show_toast(if self.reasoning_visible {
                    "Reasoning visible".to_string()
                } else {
                    "Reasoning hidden".to_string()
                });
            }
            KeyCode::Char('o') if !self.streaming && self.input.is_empty() => {
                self.toggle_collapse_last_tool();
                let has_collapsed = self.collapsed.iter().any(|&idx| {
                    self.messages.get(idx).map(|m| m.role == "tool_result" || m.role == "tool_call").unwrap_or(false)
                });
                self.show_toast(if has_collapsed {
                    "Tool output collapsed".to_string()
                } else {
                    "Tool output expanded".to_string()
                });
            }
            KeyCode::Tab if !self.autocomplete_candidates.is_empty() => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.autocomplete_idx = if self.autocomplete_idx <= 0 {
                        self.autocomplete_candidates.len() as isize - 1
                    } else {
                        self.autocomplete_idx - 1
                    };
                } else {
                    self.autocomplete_idx = (self.autocomplete_idx + 1) % self.autocomplete_candidates.len() as isize;
                }
            }
            KeyCode::Enter if !self.autocomplete_candidates.is_empty() => {
                self.apply_autocomplete();
            }
            KeyCode::Enter if !self.streaming && !key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.cancelled.store(false, Ordering::SeqCst);
                let input = std::mem::take(&mut self.input);
                self.cursor = 0;
                let msg = input.trim().to_string();
                self.autocomplete_candidates.clear();
                self.autocomplete_idx = -1;
                if !msg.is_empty() {
                    self.input_history.push(msg.clone());
                    self.history_index = -1;
                    self.saved_input.clear();
                    self.messages.push(TuiMessage {
                        age: 0,
                        role: "user".to_string(),
                        content: msg.clone(),
                    });
                    self.scroll = 0;

                    if msg.starts_with('/') {
                        self.handle_slash(&msg).await;
                        return Ok(());
                    }

                    self.prompt_count += 1;
                    self.messages.push(TuiMessage {
                        age: 0,
                        role: "assistant".to_string(),
                        content: "...".to_string(),
                    });

                    let session = self.session.clone();
                    let cancelled = self.cancelled.clone();
                    let (tx, rx) = mpsc::channel(256);
                    let (perm_tx, mut perm_rx) = mpsc::unbounded_channel();
                    self.perm_tx = perm_tx;
                    self.stream_rx = Some(rx);
                    self.streaming = true;
                    self.pending_response.clear();

                    tokio::spawn(async move {
                        let mut session = session.lock().await;
                        session.prompt_stream(&msg, tx, cancelled, &mut perm_rx).await;
                    });
                }
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.input.insert(self.cursor, '\n');
                self.cursor += 1;
            }
            KeyCode::Char('y') if self.pending_perm.is_some() => {
                if let Some(id) = self.pending_perm.take() {
                    let _ = self.perm_tx.send((id, PermissionAction::Allow));
                    if let Some(last) = self.messages.last_mut() {
                        if last.role == "tool_call" {
                            last.content = last.content.replace("(AWAITING APPROVAL)", "(approved)");
                        }
                    }
                }
            }
            KeyCode::Char('n') if self.pending_perm.is_some() => {
                if let Some(id) = self.pending_perm.take() {
                    let _ = self.perm_tx.send((id, PermissionAction::Deny));
                    if let Some(last) = self.messages.last_mut() {
                        if last.role == "tool_call" {
                            last.content = last.content.replace("(AWAITING APPROVAL)", "(denied)");
                        }
                    }
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
                if c == '@' || (c == '/' && self.cursor == 1) {
                    self.trigger_autocomplete();
                } else if !self.autocomplete_candidates.is_empty() {
                    self.trigger_autocomplete();
                }
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.input.remove(self.cursor);
                }
                if !self.autocomplete_candidates.is_empty() {
                    self.trigger_autocomplete();
                }
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor < self.input.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Home => {
                self.cursor = 0;
            }
            KeyCode::End => {
                self.cursor = self.input.len();
            }
            KeyCode::Up if !self.input_history.is_empty() => {
                if self.history_index == -1 {
                    self.saved_input = self.input.clone();
                }
                let new_idx = if self.history_index == -1 {
                    self.input_history.len() as isize - 1
                } else {
                    (self.history_index - 1).max(0)
                };
                self.history_index = new_idx;
                self.input = self.input_history[new_idx as usize].clone();
            }
            KeyCode::Down if self.history_index != -1 => {
                if self.history_index as usize + 1 < self.input_history.len() {
                    self.history_index += 1;
                    self.input = self.input_history[self.history_index as usize].clone();
                } else {
                    self.history_index = -1;
                    self.input = std::mem::take(&mut self.saved_input);
                }
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(10);
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(10);
            }
            _ => {}
        }
        Ok(())
    }

    fn toggle_collapse_last_tool(&mut self) {
        let last_tool = self.messages.iter().rposition(|m| m.role == "tool_result" || m.role == "tool_call");
        if let Some(idx) = last_tool {
            if !self.collapsed.remove(&idx) {
                self.collapsed.insert(idx);
            }
        }
    }

    fn show_toast(&mut self, msg: String) {
        self.toast = Some((msg, 6));
    }

    fn render(&mut self, f: &mut Frame) {
        let has_toast = self.toast.is_some();
        if has_toast {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(3),
                ])
                .split(f.area());
            self.render_messages(f, chunks[0]);
            if let Some((ref msg, _)) = self.toast {
                self.render_toast(f, chunks[1], msg);
            }
            self.render_status(f, chunks[2]);
            self.render_input(f, chunks[3]);
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1), Constraint::Length(3)])
                .split(f.area());
            self.render_messages(f, chunks[0]);
            self.render_status(f, chunks[1]);
            self.render_input(f, chunks[2]);
        }

        // Decrement toast counter for next frame
        if let Some((_, ref mut count)) = self.toast {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.toast = None;
            }
        }

        // Render diff viewer overlay on top of everything
        if self.diff_viewer.is_some() {
            self.render_diff_viewer(f);
        }
    }

    fn render_diff_viewer(&mut self, f: &mut Frame) {
        let (lines, scroll) = match &self.diff_viewer {
            Some(v) => v,
            None => return,
        };
        let t = self.theme;
        let max_lines = (f.area().height as usize).saturating_sub(6);
        let scroll = *scroll;

        let visible: Vec<&str> = lines.iter().skip(scroll).take(max_lines).map(|s| s.as_str()).collect();
        let total = lines.len();

        let title = format!(" Diff Viewer [{} lines] (↑↓/PgUp/PgDn scroll, Esc close) ", total);
        let items: Vec<ListItem> = visible
            .iter()
            .map(|line| {
                let style = if line.starts_with('+') && !line.starts_with("+++") {
                    Style::default().fg(t.success).add_modifier(Modifier::DIM)
                } else if line.starts_with('-') && !line.starts_with("---") {
                    Style::default().fg(t.error).add_modifier(Modifier::DIM)
                } else if line.starts_with("@@") || line.starts_with("--- ") || line.starts_with("+++ ") {
                    Style::default().fg(t.accent).add_modifier(Modifier::DIM)
                } else {
                    Style::default().fg(t.text)
                };
                ListItem::new(Line::from(vec![Span::styled(*line, style)]))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(t.primary)),
        );

        f.render_widget(list, f.area());
    }

    fn render_toast(&self, f: &mut Frame, area: Rect, msg: &str) {
        let t = self.theme;
        let text = Span::styled(
            format!(" {} ", msg),
            Style::default()
                .fg(t.success)
                .bg(t.bg)
                .add_modifier(Modifier::BOLD),
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.success));
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), inner);
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let status = if self.streaming { "streaming" } else { "idle" };
        let t = self.theme;
        let mode_tag = if self.plan_mode {
            Span::styled(
                " PLAN ",
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        };
        let left = Span::styled(
            format!(" {} ", self.model_name),
            Style::default().fg(t.primary).add_modifier(Modifier::BOLD),
        );
        let right = Span::styled(
            format!(" {}:{} | {} ", self.theme_name, self.prompt_count, status),
            Style::default().fg(if self.streaming { t.success } else { t.dim }),
        );
        let mut spans = vec![left, Span::raw(" │ "), right];
        if self.plan_mode {
            spans.push(Span::raw(" │ "));
            spans.push(mode_tag);
        }
        if self.leader_mode {
            spans.push(Span::raw(" │ "));
            spans.push(Span::styled(
                " LEADER ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ));
        }
        let line = Line::from(spans);
        let block = Block::default().borders(Borders::TOP);
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(ratatui::widgets::Paragraph::new(line), inner);
    }

    fn render_messages(&self, f: &mut Frame, area: Rect) {
        let t = self.theme;
        let items: Vec<ListItem> = self
            .messages
            .iter()
            .enumerate()
            .rev()
            .skip(self.scroll)
            .map(|(idx, m)| {
                let mut style = match m.role.as_str() {
                    "user" => Style::default().fg(t.user_msg).add_modifier(Modifier::BOLD),
                    "assistant" => Style::default().fg(t.assistant_msg),
                    "reasoning" => Style::default().fg(t.dim).add_modifier(Modifier::DIM),
                    "tool_call" => Style::default().fg(t.tool_call).add_modifier(Modifier::DIM),
                    "tool_result" => Style::default().fg(t.tool_result).add_modifier(Modifier::DIM),
                    _ => Style::default().fg(t.text),
                };
                // Fade-in: first 10 frames ramp from dim to full brightness
                if m.age < 10 && (m.role == "assistant" || m.role == "reasoning") {
                    let fade = (m.age as f32 / 10.0);
                    if fade < 0.5 {
                        style = style.add_modifier(Modifier::DIM);
                    }
                }
                let label = match m.role.as_str() {
                    "tool_call" => "tool".to_string(),
                    "tool_result" => "result".to_string(),
                    "reasoning" => "think".to_string(),
                    r => r.to_string(),
                };

                let collapsed = self.collapsed.contains(&idx);
                let display_content = if collapsed && m.content.len() > 100 {
                    let preview: String = m.content.chars().take(100).collect();
                    format!("{}... [+{} chars collapsed]", preview, m.content.len() - 100)
                } else if !collapsed && !self.reasoning_visible && m.role == "reasoning" {
                    String::new()
                } else {
                    m.content.clone()
                };

                let header_prefix = if collapsed { "+ " } else { "  " };
                let header = Span::styled(format!("{}{}> ", header_prefix, label), style);
                let mut lines = vec![Line::from(vec![header])];

                if m.role == "assistant" || m.role == "reasoning" {
                    Self::render_highlighted(&display_content, area.width as usize - 4, &mut lines);
                } else {
                    let wrapped = textwrap::fill(&display_content, area.width as usize - 4);
                    for l in wrapped.lines() {
                        lines.push(Line::from(Span::raw(format!("  {}", l))));
                    }
                }

                lines.push(Line::from(""));
                ListItem::new(lines)
            })
            .collect();

        let messages = List::new(items)
            .block(Block::default().borders(Borders::TOP).title(" Chat "));

        f.render_widget(messages, area);
    }

    fn render_highlighted(content: &str, width: usize, out: &mut Vec<Line>) {
        let t = &crate::theme::DEFAULT;
        let code_style = Style::default().fg(t.dim).add_modifier(Modifier::DIM);
        let fence_style = Style::default().fg(t.border).add_modifier(Modifier::DIM);
        let lang_style = Style::default().fg(t.tool_call);
        let text_style = Style::default().fg(t.text);
        let diff_add = Style::default().fg(t.success).add_modifier(Modifier::DIM);
        let diff_del = Style::default().fg(t.error).add_modifier(Modifier::DIM);
        let diff_hunk = Style::default().fg(t.accent).add_modifier(Modifier::DIM);

        let mut in_code = false;
        let mut code_buf = String::new();

        for line in content.lines() {
            if line.starts_with("```") {
                if in_code {
                    if !code_buf.is_empty() {
                        Self::render_code_block(&code_buf, width, fence_style, out);
                        code_buf.clear();
                    }
                    out.push(Line::from(vec![Span::styled("  ───", fence_style)]));
                    in_code = false;
                } else {
                    let lang = line.trim_start_matches("```").trim().to_string();
                    let header = if lang.is_empty() {
                        Span::styled("  ```", fence_style)
                    } else {
                        Span::styled(format!("  ```{}", lang), lang_style)
                    };
                    out.push(Line::from(vec![header]));
                    in_code = true;
                    code_buf.clear();
                }
            } else if in_code {
                code_buf.push_str(line);
                code_buf.push('\n');
            } else if line.starts_with("+++ ") || line.starts_with("--- ") {
                out.push(Line::from(vec![Span::styled(format!("  {}", line), diff_hunk)]));
            } else if line.starts_with("@@") {
                out.push(Line::from(vec![Span::styled(format!("  {}", line), diff_hunk)]));
            } else if line.starts_with('+') && !line.starts_with("+++") {
                out.push(Line::from(vec![Span::styled(format!("  {}", line), diff_add)]));
            } else if line.starts_with('-') && !line.starts_with("---") {
                out.push(Line::from(vec![Span::styled(format!("  {}", line), diff_del)]));
            } else {
                let wrapped = textwrap::fill(line, width as usize);
                for wl in wrapped.lines() {
                    out.push(Line::from(vec![Span::styled(format!("  {}", wl), text_style)]));
                }
            }
        }

        if in_code && !code_buf.is_empty() {
            Self::render_code_block(&code_buf, width, fence_style, out);
        }
    }

    fn render_code_block(code: &str, width: usize, fence_style: Style, out: &mut Vec<Line>) {
        let t = &crate::theme::DEFAULT;
        let code_style = Style::default().fg(t.dim).add_modifier(Modifier::DIM);
        let diff_add = Style::default().fg(t.success).add_modifier(Modifier::DIM);
        let diff_del = Style::default().fg(t.error).add_modifier(Modifier::DIM);
        let diff_hunk = Style::default().fg(t.accent).add_modifier(Modifier::DIM);
        for line in code.lines() {
            let style = if line.starts_with('+') && !line.starts_with("+++") {
                diff_add
            } else if line.starts_with('-') && !line.starts_with("---") {
                diff_del
            } else if line.starts_with("@@") || line.starts_with("--- ") || line.starts_with("+++ ") {
                diff_hunk
            } else {
                code_style
            };
            let wrapped = textwrap::fill(line, width.saturating_sub(2));
            for wl in wrapped.lines() {
                out.push(Line::from(vec![Span::styled(format!("  {}", wl), style)]));
            }
        }
    }

    fn render_input(&self, f: &mut Frame, area: Rect) {
        let t = self.theme;
        let title = if self.pending_perm.is_some() {
            " Approve? (y=allow / n=deny) ".to_string()
        } else if self.leader_mode {
            " Leader: (f)iles (s)essions (/)plan (t)heme (n)ew (d)iff (e)dit (m)odel (a)gent (h)elp (q)uit ".to_string()
        } else if !self.autocomplete_candidates.is_empty() {
            let idx = self.autocomplete_idx.max(0) as usize;
            let total = self.autocomplete_candidates.len();
            let preview = if idx < total {
                &self.autocomplete_candidates[idx]
            } else {
                ""
            };
            format!(
                " {} ({}/{}) {} ",
                if self.input.starts_with('/') && !self.input.contains(' ') { "Commands" } else { "@ files" },
                idx + 1, total, preview
            )
        } else {
            let hint = if self.input.contains('\n') {
                " Ctrl+Enter to send | Esc to cancel | Ctrl+R/E/O: thinking/open/collapse"
            } else {
                " Shift+Enter for newline | Ctrl+R/E/O: thinking/open/collapse"
            };
            format!(
                " Input{}{} ",
                if self.input_history.is_empty() {
                    ""
                } else {
                    "(↑↓ history) "
                },
                hint
            )
        };
        let input = Paragraph::new(self.input.as_str())
            .style(Style::default().fg(t.text))
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: true });

        f.render_widget(input, area);

        let cursor_pos = self.input.len() as u16;
        f.set_cursor_position((area.x + cursor_pos + 1, area.y + 1));
    }
}
