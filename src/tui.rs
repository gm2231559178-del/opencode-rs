use crate::llm::provider::{PermissionAction, StreamEvent};
use crate::session::Session;
use crate::session_store::SessionStore;
use crate::theme::Theme;
use anyhow::Result;
use arboard::Clipboard;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use notify_rust::Notification;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

// ── Dialog types ──────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SelectOption {
    pub title: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub value: String,
}

#[derive(Clone)]
#[allow(dead_code)]
pub enum ActiveDialog {
    Agent { options: Vec<SelectOption>, selected: usize, filter: String },
    Model { options: Vec<SelectOption>, selected: usize, filter: String },
    Theme { options: Vec<SelectOption>, selected: usize, filter: String },
    SessionList { options: Vec<SelectOption>, selected: usize, filter: String },
    MCPStatus { options: Vec<SelectOption>, selected: usize, filter: String },
    Stash { options: Vec<SelectOption>, selected: usize, filter: String },
    Skill { options: Vec<SelectOption>, selected: usize, filter: String },
    Status { options: Vec<SelectOption>, selected: usize, filter: String },
    Help,
    Alert { title: String, message: String },
    Confirm { title: String, message: String, action: String },
    CommandPalette { options: Vec<SelectOption>, selected: usize, filter: String },
    Workspace,
    Prompt { title: String, value: String, action: String, cursor: usize },
}

enum EnterAction {
    Agent(String),
    Model(String),
    Theme(String),
    SessionLoad(String),
    StashInsert(String),
    SkillInsert(String),
    CommandExecute(String),
    PromptConfirm(String, String),  // (action, value)
}

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
    pub notify: bool,
    pub model_name: String,
    pub agent_name: String,
    pub prompt_count: usize,
    pub perm_tx: mpsc::UnboundedSender<(String, PermissionAction)>,
    pub pending_perm: Option<String>,
    pub store: Option<SessionStore>,
    pub plan_mode: bool,
    pub autocomplete_candidates: Vec<String>,
    pub autocomplete_idx: isize,
    pub theme: &'static Theme,
    pub theme_name: String,
    pub reasoning: String,
    pub reasoning_visible: bool,
    pub collapsed: std::collections::HashSet<usize>,
    pub toast: Option<(String, u8)>,
    pub show_timestamps: bool,
    pub leader_mode: bool,
    pub file_watcher_rx: Option<std_mpsc::Receiver<String>>,
    pub diff_viewer: Option<(Vec<String>, usize)>,  // (lines, scroll_offset)
    pub dialog: Option<ActiveDialog>,
    pub dialog_stack: Vec<ActiveDialog>,
    pub references: Vec<crate::reference::ReferenceInfo>,
    pub frame_count: u64,
    pub sidebar_visible: bool,
    pub sidebar_panels_open: Vec<bool>,
    pub context_tokens: usize,
    pub context_percent: u8,
    pub session_cost: f64,
    pub mcp_status: Vec<(String, String)>,  // (name, status)
    pub modified_files: Vec<(String, usize, usize)>, // (path, additions, deletions)
}

#[derive(Clone)]
pub struct TuiMessage {
    pub role: String,
    pub content: String,
    pub age: u8,
    pub timestamp: chrono::DateTime<chrono::Utc>,
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
        let home = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let references = crate::reference::load_references(
            &session.config.references,
            &session.cwd,
            &home,
        );
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
            notify: true,
            model_name,
            agent_name: String::new(),
            prompt_count: 0,
            perm_tx: mpsc::unbounded_channel().0,
            pending_perm: None,
            store,
            plan_mode: false,
            autocomplete_candidates: Vec::new(),
            autocomplete_idx: -1,
            theme: &crate::theme::DEFAULT,
            theme_name: "default".to_string(),
            reasoning: String::new(),
            reasoning_visible: true,
            collapsed: std::collections::HashSet::new(),
            toast: None,
            show_timestamps: false,
            leader_mode: false,
            file_watcher_rx: None,
            diff_viewer: None,
            dialog: None,
            dialog_stack: Vec::new(),
            references,
            frame_count: 0,
            sidebar_visible: false,
            sidebar_panels_open: vec![true; 5],
            context_tokens: 0,
            context_percent: 0,
            session_cost: 0.0,
            mcp_status: Vec::new(),
            modified_files: Vec::new(),
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
        use notify::{Config, Event, RecursiveMode, Watcher};
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
                        self.context_tokens += delta.len() / 4;
                        self.context_percent = ((self.context_tokens as f64 / 100000.0) * 100.0) as u8;
                    }
                    StreamEvent::Text { delta } => {
                        self.context_tokens += delta.len() / 4;
                        self.context_percent = ((self.context_tokens as f64 / 100000.0) * 100.0) as u8;
                        let needs_new = self.messages.last().map(|m| m.role != "assistant").unwrap_or(true);
                        if needs_new {
                            self.messages.push(TuiMessage {
                                age: 0, timestamp: chrono::Utc::now(),
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
                        self.context_tokens += args_str.len() / 4;
                        self.context_percent = ((self.context_tokens as f64 / 100000.0) * 100.0) as u8;
                        let preview: String = args_str.chars().take(400).collect();
                        let icon = crate::util::tool_display::tool_icon(&name);
                        let hname = crate::util::tool_display::human_name(&name);
                        self.messages.push(TuiMessage {
            age: 0, timestamp: chrono::Utc::now(),
                            role: "tool_call".to_string(),
                            content: format!("{} {} ({})\n{}", icon, hname, short, preview),
                        });
                    }
                    StreamEvent::PermissionRequest { request_id, tool_name, args } => {
                        let args_str = serde_json::to_string_pretty(&args).unwrap_or_default();
                        let preview: String = args_str.chars().take(200).collect();
                        self.messages.push(TuiMessage {
            age: 0, timestamp: chrono::Utc::now(),
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
            age: 0, timestamp: chrono::Utc::now(),
                            role: "tool_result".to_string(),
                            content,
                        });
                    }
                    StreamEvent::Done { response } => {
                        if self.notify {
                            send_notification("OpenCode", "Response complete");
                        }

                        if !self.reasoning.is_empty() {
                            let reasoning = std::mem::take(&mut self.reasoning);
                            self.messages.push(TuiMessage {
                                age: 0, timestamp: chrono::Utc::now(),
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
                                    age: 0, timestamp: chrono::Utc::now(),
                                    role: "assistant".to_string(),
                                    content: response.trim().to_string(),
                                });
                            }
                        }
                        self.pending_response.clear();
                        done = true;
                        if self.context_tokens > 50000 {
                            let removed = {
                                let guard = self.session.try_lock();
                                match guard {
                                    Ok(mut s) => s.compact_messages(),
                                    Err(_) => 0,
                                }
                            };
                            if removed > 0 {
                                self.context_tokens = self.context_tokens / 2;
                                self.toast = Some((format!("Auto-compacted: removed {} messages", removed), 80));
                            }
                        }
                    }
                    StreamEvent::Error { message } => {
                        if self.notify {
                            send_notification("OpenCode Error", &message);
                        }
                        let updated = self.messages.iter_mut().rev().find(|m| m.role == "assistant");
                        if let Some(msg) = updated {
                            msg.content = format!("Error: {}", message);
                        } else {
                            self.messages.push(TuiMessage {
            age: 0, timestamp: chrono::Utc::now(),
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
        // Sticky scroll: when scrolled up, keep view stable as new messages arrive
        if self.scroll > 0 {
            self.scroll = self.scroll.saturating_add(1);
        }
    }

    // ── Sidebar helpers ─────────────────────────────────────

    fn refresh_sidebar_data(&mut self) {
        // Refresh modified files from git
        self.modified_files = self.get_modified_files();

        // Refresh MCP status from session config
        if let Ok(session) = self.session.try_lock() {
            let names: Vec<String> = session.config.mcp.keys().cloned().collect();
            if !names.is_empty() && self.mcp_status.is_empty() {
                // Start with "connecting" status for configured servers
                for n in names {
                    if !self.mcp_status.iter().any(|(name, _)| name == &n) {
                        self.mcp_status.push((n, "connecting".to_string()));
                    }
                }
            }
        }
    }

    fn get_modified_files(&self) -> Vec<(String, usize, usize)> {
        if let Ok(session) = self.session.try_lock() {
            let cwd = &session.cwd;
            let output = std::process::Command::new("git")
                .args(["diff", "--stat"])
                .current_dir(cwd)
                .output();
            if let Ok(out) = output {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let mut files = Vec::new();
                    for line in stdout.lines() {
                        let line = line.trim();
                        // Parse " path/to/file | 10 ++++++-------" or similar
                        if let Some(pipe_idx) = line.rfind('|') {
                            let path = line[..pipe_idx].trim().to_string();
                            let rest = line[pipe_idx + 1..].trim();
                            let add = rest.chars().filter(|&c| c == '+').count();
                            let del = rest.chars().filter(|&c| c == '-').count();
                            if !path.is_empty() {
                                files.push((path, add, del));
                            }
                        }
                    }
                    return files;
                }
            }
        }
        Vec::new()
    }

    // ── Dialog helpers ──────────────────────────────────────

    fn push_dialog(&mut self, dialog: ActiveDialog) {
        if self.dialog.is_some() {
            if let Some(old) = self.dialog.take() {
                self.dialog_stack.push(old);
            }
        }
        self.dialog = Some(dialog);
    }

    fn pop_dialog(&mut self) {
        self.dialog = self.dialog_stack.pop();
    }

    fn show_help_dialog(&mut self) {
        self.push_dialog(ActiveDialog::Help);
    }

    #[allow(dead_code)]
    fn show_alert(&mut self, title: String, message: String) {
        self.push_dialog(ActiveDialog::Alert { title, message });
    }

    fn show_confirm(&mut self, title: String, message: String, action: String) {
        self.push_dialog(ActiveDialog::Confirm { title, message, action });
    }

    fn show_prompt(&mut self, title: String, action: String, initial: String) {
        let cursor = initial.len();
        self.push_dialog(ActiveDialog::Prompt { title, value: initial, action, cursor });
    }

    fn build_agent_options(&self) -> Vec<SelectOption> {
        let mut options = Vec::new();
        if let Ok(session) = self.session.try_lock() {
            for (name, _) in &session.config.agent {
                options.push(SelectOption {
                    title: name.clone(),
                    description: Some("configured agent".to_string()),
                    category: Some("Agents".to_string()),
                    value: name.clone(),
                });
            }
        }
        if options.is_empty() {
            options.push(SelectOption {
                title: "No agents configured".to_string(),
                description: Some("Use /agent <name> to create one".to_string()),
                category: None,
                value: String::new(),
            });
        }
        options
    }

    fn build_theme_options(&self) -> Vec<SelectOption> {
        let names = ["default", "tokyonight", "catppuccin", "gruvbox", "dracula", "nord", "onedark"];
        names.iter().map(|name| SelectOption {
            title: name.to_string(),
            description: if *name == self.theme_name { Some("current".to_string()) } else { None },
            category: Some("Themes".to_string()),
            value: name.to_string(),
        }).collect()
    }

    fn build_model_options(&self) -> Vec<SelectOption> {
        let current_model = self.model_name.clone();
        let models = vec![
            "openai/gpt-4o", "openai/gpt-4o-mini", "openai/o1", "openai/o3-mini",
            "anthropic/claude-sonnet-4-20250514", "anthropic/claude-3-5-haiku-latest",
            "openrouter/anthropic/claude-sonnet-4", "openrouter/openai/gpt-4o",
            "groq/llama-3.3-70b-versatile", "groq/deepseek-r1-distill-llama-70b",
            "opencode/default",
        ];
        models.iter().map(|m| SelectOption {
            title: m.to_string(),
            description: if *m == current_model { Some("current".to_string()) } else { None },
            category: Some(m.split('/').next().unwrap_or("other").to_string()),
            value: m.to_string(),
        }).collect()
    }

    fn build_session_options(&self) -> Vec<SelectOption> {
        let mut options = Vec::new();
        if let Some(store) = &self.store {
            if let Ok(sessions) = store.list_sessions(50) {
                for s in &sessions {
                    let preview = if s.id.len() > 8 { &s.id[..8] } else { &s.id };
                    options.push(SelectOption {
                        title: format!("{} | {} | {} msgs", preview, s.model, s.message_count),
                        description: Some(format!("Updated: {}", s.updated_at)),
                        category: Some("Sessions".to_string()),
                        value: s.id.clone(),
                    });
                }
            }
        }
        if options.is_empty() {
            options.push(SelectOption {
                title: "No saved sessions".to_string(),
                description: None,
                category: None,
                value: String::new(),
            });
        }
        options
    }

    fn build_mcp_options(&self) -> Vec<SelectOption> {
        let mut options = Vec::new();
        if let Ok(session) = self.session.try_lock() {
            let tool_count = session.tools.len();
            let mcp_tools: Vec<&str> = session.tools.iter()
                .filter(|t| t.name().starts_with("mcp_"))
                .map(|t| t.name())
                .collect();
            if mcp_tools.is_empty() {
                options.push(SelectOption {
                    title: "No MCP tools connected".to_string(),
                    description: Some("Configure MCP servers in opencode.jsonc".to_string()),
                    category: None,
                    value: String::new(),
                });
            } else {
                options.push(SelectOption {
                    title: format!("{} total tools ({} MCP)", tool_count, mcp_tools.len()),
                    description: None,
                    category: Some("MCP Tools".to_string()),
                    value: "summary".to_string(),
                });
                for t in mcp_tools {
                    options.push(SelectOption {
                        title: t.to_string(),
                        description: None,
                        category: None,
                        value: t.to_string(),
                    });
                }
            }
        }
        options
    }

    fn build_stash_options(&self) -> Vec<SelectOption> {
        // Stash provides quick-access saved prompts
        let stashed = vec![
            ("help", "List available commands"),
            ("plan", "Toggle plan mode"),
            ("compact", "Compact conversation"),
            ("stats", "Show usage statistics"),
            ("diagnostics", "Run diagnostics on current file"),
        ];
        stashed.iter().map(|(name, desc)| SelectOption {
            title: name.to_string(),
            description: Some(desc.to_string()),
            category: Some("Stashed prompts".to_string()),
            value: format!("/{}", name),
        }).collect()
    }

    #[allow(dead_code)]
    fn build_skill_options(&self) -> Vec<SelectOption> {
        let mut options = Vec::new();
        if let Ok(session) = self.session.try_lock() {
            for (name, _) in &session.config.agent {
                options.push(SelectOption {
                    title: name.clone(),
                    description: Some("available skill".to_string()),
                    category: Some("Skills".to_string()),
                    value: name.clone(),
                });
            }
        }
        if options.is_empty() {
            options.push(SelectOption {
                title: "No skills configured".to_string(),
                description: None,
                category: None,
                value: String::new(),
            });
        }
        options
    }

    fn build_status_options(&self) -> Vec<SelectOption> {
        let mut options = Vec::new();
        options.push(SelectOption {
            title: format!("Model: {}", self.model_name),
            description: None,
            category: Some("Session".to_string()),
            value: "model".to_string(),
        });
        options.push(SelectOption {
            title: format!("Theme: {}", self.theme_name),
            description: None,
            category: None,
            value: "theme".to_string(),
        });
        options.push(SelectOption {
            title: format!("Plan mode: {}", if self.plan_mode { "ON" } else { "OFF" }),
            description: None,
            category: None,
            value: "plan".to_string(),
        });
        options.push(SelectOption {
            title: format!("Notifications: {}", if self.notify { "ON" } else { "OFF" }),
            description: None,
            category: None,
            value: "notify".to_string(),
        });
        if let Ok(session) = self.session.try_lock() {
            let s = &session.stats;
            options.push(SelectOption {
                title: format!("Prompts: {} | Tokens: {}", s.prompt_count, s.total_tokens),
                description: None,
                category: Some("Stats".to_string()),
                value: "stats".to_string(),
            });
        }
        options
    }

    fn build_command_palette_options(&self) -> Vec<SelectOption> {
        let mut options = Vec::new();
        // Navigation & session
        options.push(SelectOption { title: "New session".into(), description: Some("Clear current session and start fresh".into()), category: Some("Session".into()), value: "new".into() });
        options.push(SelectOption { title: "Toggle plan mode".into(), description: Some("Read-only planning mode".into()), category: Some("Session".into()), value: "plan".into() });
        options.push(SelectOption { title: "Compact conversation".into(), description: Some("Summarize and trim history".into()), category: Some("Session".into()), value: "compact".into() });
        options.push(SelectOption { title: "Session list".into(), description: Some("Browse and load saved sessions".into()), category: Some("Session".into()), value: "sessions".into() });
        options.push(SelectOption { title: "Rename session".into(), description: Some("Change the session title".into()), category: Some("Session".into()), value: "rename".into() });
        options.push(SelectOption { title: "Delete session".into(), description: Some("Permanently delete current session".into()), category: Some("Session".into()), value: "delete_session".into() });
        options.push(SelectOption { title: "Undo last file change".into(), description: None, category: Some("Session".into()), value: "undo".into() });
        // Display
        options.push(SelectOption { title: "Toggle sidebar".into(), description: Some("Show or hide the sidebar panel".into()), category: Some("Display".into()), value: "sidebar".into() });
        options.push(SelectOption { title: "Toggle reasoning".into(), description: Some("Show or hide reasoning blocks".into()), category: Some("Display".into()), value: "reasoning".into() });
        options.push(SelectOption { title: "Toggle collapse".into(), description: Some("Collapse or expand tool output".into()), category: Some("Display".into()), value: "collapse".into() });
        options.push(SelectOption { title: "Switch theme".into(), description: Some("Change color theme".into()), category: Some("Display".into()), value: "theme".into() });
        options.push(SelectOption { title: "Diff viewer".into(), description: Some("Show file diff overlay".into()), category: Some("Display".into()), value: "diff".into() });
        // Model & agent
        options.push(SelectOption { title: "Switch model".into(), description: Some("Change the LLM model".into()), category: Some("Model".into()), value: "model".into() });
        options.push(SelectOption { title: "Switch agent".into(), description: Some("Change the active agent".into()), category: Some("Model".into()), value: "agent".into() });
        options.push(SelectOption { title: "Show help".into(), description: Some("Display key bindings reference".into()), category: Some("Info".into()), value: "help".into() });
        options.push(SelectOption { title: "Session status".into(), description: Some("Show current session info".into()), category: Some("Info".into()), value: "status".into() });
        options.push(SelectOption { title: "MCP status".into(), description: Some("Show MCP tool connections".into()), category: Some("Info".into()), value: "mcp".into() });
        options.push(SelectOption { title: "Show version".into(), description: None, category: Some("Info".into()), value: "version".into() });
        options.push(SelectOption { title: "Show agents file".into(), description: Some("Display AGENTS.md workspace instructions".into()), category: Some("Info".into()), value: "agents".into() });
        // Actions
        options.push(SelectOption { title: "Edit last file".into(), description: Some("Open last edited file in $EDITOR".into()), category: Some("Actions".into()), value: "edit".into() });
        options.push(SelectOption { title: "Copy last response".into(), description: Some("Copy last assistant response to clipboard".into()), category: Some("Actions".into()), value: "copy".into() });
        options.push(SelectOption { title: "Quit".into(), description: Some("Exit OpenCode".into()), category: Some("Actions".into()), value: "quit".into() });
        options
    }

    fn filter_options(options: &[SelectOption], filter: &str) -> Vec<SelectOption> {
        if filter.is_empty() {
            return options.to_vec();
        }
        let lower = filter.to_lowercase();
        options.iter()
            .filter(|o| o.title.to_lowercase().contains(&lower) || o.description.as_deref().unwrap_or("").to_lowercase().contains(&lower))
            .cloned()
            .collect()
    }

    fn handle_dialog_key(&mut self, key: crossterm::event::KeyEvent) -> Result<bool> {
        if self.dialog.is_none() {
            return Ok(false);
        }

        // Special handling for Confirm dialog — y/n keys
        if let Some(ActiveDialog::Confirm { ref action, .. }) = self.dialog {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    let act = action.clone();
                    self.pop_dialog();
                    self.exec_confirm_action(&act);
                    return Ok(true);
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.pop_dialog();
                    return Ok(true);
                }
                _ => {}
            }
        }

        // Special handling for Prompt dialog — intercept text input
        if let Some(ActiveDialog::Prompt { ref mut value, ref mut cursor, .. }) = self.dialog {
            match key.code {
                KeyCode::Esc => {
                    self.pop_dialog();
                    return Ok(true);
                }
                KeyCode::Enter => {
                    if let Some(d) = self.dialog.take() {
                        if let ActiveDialog::Prompt { action, value, .. } = d {
                            self.execute_dialog_action(EnterAction::PromptConfirm(action, value));
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Char(c) => {
                    value.push(c);
                    *cursor = value.len();
                    return Ok(true);
                }
                KeyCode::Backspace => {
                    value.pop();
                    *cursor = value.len();
                    return Ok(true);
                }
                _ => return Ok(true),
            }
        }

        use ActiveDialog::*;
        match key.code {
            KeyCode::Esc => {
                match self.dialog.as_ref().unwrap() {
                    Help | Alert { .. } | Agent { .. } | Model { .. } | Theme { .. }
                    | SessionList { .. } | MCPStatus { .. } | Stash { .. } | Skill { .. }
                    | Status { .. } | Confirm { .. } | CommandPalette { .. } | Workspace | Prompt { .. } => {
                        self.pop_dialog();
                    }
                }
                Ok(true)
            }
            KeyCode::Enter => {
                let action = match self.dialog.as_ref().unwrap() {
                    Help | Alert { .. } | Workspace => {
                        self.pop_dialog();
                        None
                    }
                    Confirm { action, .. } => {
                        let act = action.clone();
                        self.pop_dialog();
                        self.exec_confirm_action(&act);
                        None
                    }
                    Agent { options, selected, filter } => {
                        let filtered = Self::filter_options(options, filter);
                        if *selected < filtered.len() && !filtered[*selected].value.is_empty() {
                            Some(EnterAction::Agent(filtered[*selected].value.clone()))
                        } else {
                            None
                        }
                    }
                    Model { options, selected, filter } => {
                        let filtered = Self::filter_options(options, filter);
                        if *selected < filtered.len() && !filtered[*selected].value.is_empty() {
                            Some(EnterAction::Model(filtered[*selected].value.clone()))
                        } else {
                            None
                        }
                    }
                    Theme { options, selected, filter } => {
                        let filtered = Self::filter_options(options, filter);
                        if *selected < filtered.len() && !filtered[*selected].value.is_empty() {
                            Some(EnterAction::Theme(filtered[*selected].value.clone()))
                        } else {
                            None
                        }
                    }
                    SessionList { options, selected, filter } => {
                        let filtered = Self::filter_options(options, filter);
                        if *selected < filtered.len() && !filtered[*selected].value.is_empty() {
                            Some(EnterAction::SessionLoad(filtered[*selected].value.clone()))
                        } else {
                            None
                        }
                    }
                    Stash { options, selected, filter } => {
                        let filtered = Self::filter_options(options, filter);
                        if *selected < filtered.len() && !filtered[*selected].value.is_empty() {
                            Some(EnterAction::StashInsert(filtered[*selected].value.clone()))
                        } else {
                            None
                        }
                    }
                    Skill { options, selected, filter } => {
                        let filtered = Self::filter_options(options, filter);
                        if *selected < filtered.len() && !filtered[*selected].value.is_empty() {
                            Some(EnterAction::SkillInsert(filtered[*selected].value.clone()))
                        } else {
                            None
                        }
                    }
                    MCPStatus { .. } | Status { .. } => None,
                    CommandPalette { options, selected, filter } => {
                        let filtered = Self::filter_options(options, filter);
                        if *selected < filtered.len() && !filtered[*selected].value.is_empty() {
                            Some(EnterAction::CommandExecute(filtered[*selected].value.clone()))
                        } else {
                            None
                        }
                    }
                    Prompt { title: _, value, action, cursor: _ } => {
                        let val = value.clone();
                        let act = action.clone();
                        Some(EnterAction::PromptConfirm(act, val))
                    }
                };
                if let Some(action) = action {
                    self.execute_dialog_action(action);
                    self.dialog = None;
                }
                Ok(true)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.dialog_select_move(-1);
                Ok(true)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.dialog_select_move(1);
                Ok(true)
            }
            KeyCode::Char(c) => {
                self.dialog_select_filter_push(c);
                Ok(true)
            }
            KeyCode::Backspace => {
                self.dialog_select_filter_pop();
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    fn execute_dialog_action(&mut self, action: EnterAction) {
        match action {
            EnterAction::Agent(name) => { self.cmd_set_agent(name); }
            EnterAction::Model(name) => { self.cmd_set_model(name); }
            EnterAction::Theme(name) => {
                self.theme = crate::theme::Theme::by_name(&name);
                self.theme_name = name;
            }
            EnterAction::SessionLoad(id) => { self.cmd_load_session(&id); }
            EnterAction::StashInsert(cmd) => {
                self.input = cmd;
                self.cursor = self.input.len();
            }
            EnterAction::SkillInsert(name) => {
                self.input = format!("/skill {}", name);
                self.cursor = self.input.len();
            }
            EnterAction::CommandExecute(cmd) => {
                self.exec_command_palette_action(&cmd);
            }
            EnterAction::PromptConfirm(action, value) => {
                self.exec_prompt_action(&action, &value);
            }
        }
    }

    fn exec_confirm_action(&mut self, action: &str) {
        if action == "clear" {
            self.cmd_clear_session();
            if let Ok(mut s) = self.session.try_lock() {
                s.id = uuid::Uuid::new_v4().to_string();
            }
            self.show_toast("New session created.".to_string());
        } else if let Some(id) = action.strip_prefix("delete_session:") {
            if let Some(store) = &self.store {
                match store.delete_session(id) {
                    Ok(()) => self.show_toast(format!("Session {} deleted.", &id[..8.min(id.len())])),
                    Err(e) => self.show_toast(format!("Delete failed: {}", e)),
                }
            }
        }
    }

    fn exec_prompt_action(&mut self, action: &str, value: &str) {
        match action {
            "rename_session" => {
                if value.is_empty() { return; }
                let id = self.session.try_lock().map(|s| s.id.clone()).unwrap_or_default();
                if let Some(store) = &self.store {
                    if let Err(e) = store.rename_session(&id, value) {
                        self.show_toast(format!("Rename failed: {}", e));
                        return;
                    }
                }
                self.show_toast(format!("Session renamed to: {}", value));
            }
            _ => {
                self.show_toast(format!("Prompt action '{}' not implemented", action));
            }
        }
    }

    fn exec_command_palette_action(&mut self, cmd: &str) {
        match cmd {
            "new" => {
                self.show_confirm("New Session".to_string(), "Clear current session?".to_string(), "clear".to_string());
            }
            "plan" => {
                self.plan_mode = !self.plan_mode;
                self.show_toast(format!("Plan mode: {}", if self.plan_mode { "ON" } else { "OFF" }));
            }
            "compact" => {
                self.input = "/compact".to_string();
                self.cursor = self.input.len();
            }
            "sessions" => {
                self.push_dialog(ActiveDialog::SessionList {
                    options: self.build_session_options(),
                    selected: 0,
                    filter: String::new(),
                });
            }
            "undo" => {
                self.input = "/undo".to_string();
                self.cursor = self.input.len();
            }
            "sidebar" => {
                self.sidebar_visible = !self.sidebar_visible;
                self.show_toast(if self.sidebar_visible { "Sidebar shown".to_string() } else { "Sidebar hidden".to_string() });
            }
            "reasoning" => {
                self.reasoning_visible = !self.reasoning_visible;
                self.show_toast(if self.reasoning_visible { "Reasoning visible".to_string() } else { "Reasoning hidden".to_string() });
            }
            "collapse" => {
                self.toggle_collapse_last_tool();
            }
            "theme" => {
                self.push_dialog(ActiveDialog::Theme {
                    options: self.build_theme_options(),
                    selected: 0,
                    filter: String::new(),
                });
            }
            "diff" => {
                self.input = "/diff".to_string();
                self.cursor = self.input.len();
            }
            "model" => {
                self.push_dialog(ActiveDialog::Model {
                    options: self.build_model_options(),
                    selected: 0,
                    filter: String::new(),
                });
            }
            "agent" => {
                self.push_dialog(ActiveDialog::Agent {
                    options: self.build_agent_options(),
                    selected: 0,
                    filter: String::new(),
                });
            }
            "help" => {
                self.show_help_dialog();
            }
            "status" => {
                self.push_dialog(ActiveDialog::Status {
                    options: self.build_status_options(),
                    selected: 0,
                    filter: String::new(),
                });
            }
            "mcp" => {
                self.push_dialog(ActiveDialog::MCPStatus {
                    options: self.build_mcp_options(),
                    selected: 0,
                    filter: String::new(),
                });
            }
            "version" => {
                self.input = "/version".to_string();
                self.cursor = self.input.len();
            }
            "agents" => {
                self.input = "/agents".to_string();
                self.cursor = self.input.len();
            }
            "edit" => {
                self.open_last_edited_file();
            }
            "copy" => {
                self.copy_last_response();
                self.show_toast("Copied last response to clipboard".to_string());
            }
            "rename" => {
                let current_title = self.session.try_lock().map(|s| s.id.clone()).unwrap_or_default();
                self.show_prompt("Rename Session".to_string(), "rename_session".to_string(), current_title);
            }
            "delete_session" => {
                let id = self.session.try_lock().map(|s| s.id.clone()).unwrap_or_default();
                let short = if id.len() > 8 { &id[..8] } else { &id };
                self.show_confirm(
                    "Delete Session".to_string(),
                    format!("Permanently delete session {}? This cannot be undone.", short),
                    format!("delete_session:{}", id),
                );
            }
            "quit" => {
                self.quit = true;
            }
            _ => {
                // Try as a slash command
                if !cmd.starts_with('/') {
                    self.input = format!("/{}", cmd);
                } else {
                    self.input = cmd.to_string();
                }
                self.cursor = self.input.len();
            }
        }
    }

    fn dialog_select_move(&mut self, delta: isize) {
        let dialog = match self.dialog.as_mut() {
            Some(d) => d,
            None => return,
        };
        use ActiveDialog::*;
        match dialog {
            Agent { options, selected, filter }
            | Model { options, selected, filter }
            | Theme { options, selected, filter }
            | SessionList { options, selected, filter }
            | MCPStatus { options, selected, filter }
            | Stash { options, selected, filter }
            | Skill { options, selected, filter }
            | Status { options, selected, filter }
            | CommandPalette { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let len = filtered.len();
                if len == 0 { return; }
                let new_idx = (*selected as isize + delta).rem_euclid(len as isize) as usize;
                *selected = new_idx;
            }
            _ => {}
        }
    }

    fn dialog_select_filter_push(&mut self, c: char) {
        let dialog = match self.dialog.as_mut() {
            Some(d) => d,
            None => return,
        };
        use ActiveDialog::*;
        match dialog {
            Agent { filter, selected, .. }
            | Model { filter, selected, .. }
            | Theme { filter, selected, .. }
            | SessionList { filter, selected, .. }
            | MCPStatus { filter, selected, .. }
            | Stash { filter, selected, .. }
            | Skill { filter, selected, .. }
            | Status { filter, selected, .. } => {
                filter.push(c);
                *selected = 0;
            }
            _ => {}
        }
    }

    fn dialog_select_filter_pop(&mut self) {
        let dialog = match self.dialog.as_mut() {
            Some(d) => d,
            None => return,
        };
        use ActiveDialog::*;
        match dialog {
            Agent { filter, selected, .. }
            | Model { filter, selected, .. }
            | Theme { filter, selected, .. }
            | SessionList { filter, selected, .. }
            | MCPStatus { filter, selected, .. }
            | Stash { filter, selected, .. }
            | Skill { filter, selected, .. }
            | Status { filter, selected, .. } => {
                filter.pop();
                *selected = 0;
            }
            _ => {}
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
                age: 0, timestamp: chrono::Utc::now(),
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
                        self.agent_name = name.clone();
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
                                age: 0, timestamp: chrono::Utc::now(),
                                role: "assistant".to_string(),
                                content: "Last response copied to clipboard.".to_string(),
                            });
                        } else {
                            self.messages.push(TuiMessage {
                                age: 0, timestamp: chrono::Utc::now(),
                                role: "assistant".to_string(),
                                content: "Failed to copy to clipboard.".to_string(),
                            });
                        }
                    }
                    Err(_) => {
                        self.messages.push(TuiMessage {
                            age: 0, timestamp: chrono::Utc::now(),
                            role: "assistant".to_string(),
                            content: "Clipboard not available.".to_string(),
                        });
                    }
                }
            }
            _ => {
                self.messages.push(TuiMessage {
                    age: 0, timestamp: chrono::Utc::now(),
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

        // @ file & reference autocomplete
        let at_pos = before_cursor.rfind('@');
        match at_pos {
            Some(pos) => {
                let query = before_cursor[pos + 1..].to_string();
                let file_query = query.split('#').next().unwrap_or(&query).to_string();

                // File candidates via fd
                let mut candidates: Vec<String> = Vec::new();
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
                let mut file_candidates: Vec<String> = output
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.lines().map(|l| l.to_string()).collect())
                    .unwrap_or_default();
                if !file_query.is_empty() {
                    file_candidates.sort_by_key(|c| {
                        let lower_c = c.to_lowercase();
                        let lower_q = file_query.to_lowercase();
                        lower_c.find(&lower_q).unwrap_or(usize::MAX)
                    });
                }
                for c in &mut file_candidates {
                    let path = std::path::Path::new(c);
                    if path.is_dir() {
                        c.push('/');
                    }
                }

                // Reference candidates
                let ref_candidates: Vec<String> = self.references.iter()
                    .filter(|r| r.name.to_lowercase().contains(&file_query.to_lowercase()))
                    .map(|r| format!("ref:{}", r.name))
                    .collect();

                // Combine: files first, then references
                candidates.extend(file_candidates);
                candidates.extend(ref_candidates);

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

            // Determine the display name (strip ref: prefix for references)
            let display_name = if let Some(ref_name) = selected.strip_prefix("ref:") {
                ref_name.to_string()
            } else {
                selected.clone()
            };

            let replacement = if suffix.is_empty() {
                format!("{} ", display_name)
            } else {
                display_name
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
        // Dialog mode handles all keys when active
        if self.dialog.is_some() {
            self.handle_dialog_key(key)?;
            return Ok(());
        }

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
            match key.code {
                KeyCode::Char('b') => {
                    self.sidebar_visible = !self.sidebar_visible;
                    self.show_toast(if self.sidebar_visible { "Sidebar shown".to_string() } else { "Sidebar hidden".to_string() });
                }
                KeyCode::Char('k') => {
                    self.push_dialog(ActiveDialog::CommandPalette {
                        options: self.build_command_palette_options(),
                        selected: 0,
                        filter: String::new(),
                    });
                }
                KeyCode::Char('f') => {
                    self.input = "/diagnostics ".to_string();
                    self.cursor = self.input.len();
                }
                KeyCode::Char('s') => {
                    self.push_dialog(ActiveDialog::SessionList {
                        options: self.build_session_options(),
                        selected: 0,
                        filter: String::new(),
                    });
                }
                KeyCode::Char('m') => {
                    self.push_dialog(ActiveDialog::Model {
                        options: self.build_model_options(),
                        selected: 0,
                        filter: String::new(),
                    });
                }
                KeyCode::Char('a') => {
                    self.push_dialog(ActiveDialog::Agent {
                        options: self.build_agent_options(),
                        selected: 0,
                        filter: String::new(),
                    });
                }
                KeyCode::Char('t') => {
                    self.push_dialog(ActiveDialog::Theme {
                        options: self.build_theme_options(),
                        selected: 0,
                        filter: String::new(),
                    });
                }
                KeyCode::Char('h') => {
                    self.show_help_dialog();
                }
                KeyCode::Char('p') => {
                    self.push_dialog(ActiveDialog::Stash {
                        options: self.build_stash_options(),
                        selected: 0,
                        filter: String::new(),
                    });
                }
                KeyCode::Char('c') => {
                    self.push_dialog(ActiveDialog::MCPStatus {
                        options: self.build_mcp_options(),
                        selected: 0,
                        filter: String::new(),
                    });
                }
                KeyCode::Char('/') => {
                    self.input = "/plan".to_string();
                    self.cursor = self.input.len();
                    self.handle_slash("/plan").await;
                }
                KeyCode::Char('n') => {
                    self.show_confirm("New Session".to_string(), "Clear current session?".to_string(), "clear".to_string());
                }
                KeyCode::Char('d') => {
                    self.input = "/diff".to_string();
                    self.cursor = self.input.len();
                    self.handle_slash("/diff").await;
                }
                KeyCode::Char('e') => {
                    self.open_last_edited_file();
                }
                KeyCode::Char('q') => { self.quit = true; }
                KeyCode::Char('w') => {
                    self.push_dialog(ActiveDialog::Workspace);
                }
                KeyCode::Char('r') => {
                    let current_title = self.session.try_lock().map(|s| s.id.clone()).unwrap_or_default();
                    self.show_prompt("Rename Session".to_string(), "rename_session".to_string(), current_title);
                }
                KeyCode::Char('?') => {
                    self.push_dialog(ActiveDialog::Status {
                        options: self.build_status_options(),
                        selected: 0,
                        filter: String::new(),
                    });
                }
                KeyCode::Esc => {}
                _ => { self.show_toast("Unknown leader key".to_string()); }
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
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) && self.input.is_empty() => {
                self.sidebar_visible = !self.sidebar_visible;
                self.show_toast(if self.sidebar_visible { "Sidebar shown".to_string() } else { "Sidebar hidden".to_string() });
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) && self.input.is_empty() => {
                self.push_dialog(ActiveDialog::CommandPalette {
                    options: self.build_command_palette_options(),
                    selected: 0,
                    filter: String::new(),
                });
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
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) && !self.streaming => {
                self.show_timestamps = !self.show_timestamps;
                self.show_toast(if self.show_timestamps {
                    "Timestamps visible".to_string()
                } else {
                    "Timestamps hidden".to_string()
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
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                let c = self.cursor;
                self.input.insert(c, '\n');
                self.cursor = c + 1;
            }
            KeyCode::Enter if !self.streaming => {
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
                        age: 0, timestamp: chrono::Utc::now(),
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
                        age: 0, timestamp: chrono::Utc::now(),
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
        self.frame_count = self.frame_count.wrapping_add(1);
        let has_toast = self.toast.is_some();
        let has_ac = !self.autocomplete_candidates.is_empty();
        let ac_count = self.autocomplete_candidates.len();
        let ac_height = if has_ac { (ac_count + 1).min(10) as u16 } else { 0 };

        // When sidebar is visible, split horizontally — main left, sidebar right
        let main_area = if self.sidebar_visible {
            let horiz = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(1), Constraint::Length(42)])
                .split(f.area());
            self.render_sidebar(f, horiz[1]);
            horiz[0]
        } else {
            f.area()
        };

        // Refresh sidebar data periodically
        if self.sidebar_visible {
            self.refresh_sidebar_data();
        }

        // Build vertical constraints
        let mut constraints = Vec::new();
        constraints.push(Constraint::Min(1)); // messages
        constraints.push(Constraint::Length(5)); // input

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(main_area);

        let mut ci = 0;
        self.render_messages(f, chunks[ci]);
        // Toast rendered as overlay on top of messages
        if has_toast {
            if let Some((ref msg, _)) = self.toast {
                self.render_toast_overlay(f, chunks[ci], msg);
            }
        }
        ci += 1;
        self.render_input(f, chunks[ci]);

        // Autocomplete rendered as overlay above the input bar
        if has_ac {
            let ac_area = Rect {
                x: main_area.x,
                y: chunks[ci].y.saturating_sub(ac_height),
                width: main_area.width,
                height: ac_height,
            };
            f.render_widget(Clear, ac_area);
            self.render_autocomplete_popup(f, ac_area, &self.autocomplete_candidates, self.autocomplete_idx);
        }

        // Decrement toast counter for next frame
        if let Some((_, ref mut count)) = self.toast {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.toast = None;
            }
        }

        // Render diff viewer overlay
        if self.diff_viewer.is_some() {
            self.render_diff_viewer(f);
        }

        // Render dialog overlay on top of everything
        if self.dialog.is_some() {
            self.render_dialog(f);
        }
    }

    fn render_sidebar(&self, f: &mut Frame, area: Rect) {
        let t = self.theme;
        let inner_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width.min(36),
            height: area.height,
        };

        // Background panel
        let panel = Block::default()
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(t.border))
            .style(Style::default().bg(t.background_panel));
        f.render_widget(panel, inner_area);

        let content_area = Rect {
            x: inner_area.x + 1,
            y: inner_area.y,
            width: inner_area.width.saturating_sub(2),
            height: inner_area.height,
        };

        let section_style = Style::default().fg(t.text).add_modifier(Modifier::BOLD);
        let muted = Style::default().fg(t.text_muted);
        let green = Style::default().fg(t.success);
        let red = Style::default().fg(t.error);
        let yellow = Style::default().fg(t.warning);

        let mut lines: Vec<Line> = Vec::new();
        let w = content_area.width as usize;

        // ── Section 0: Context ──────────────────────────────
        {
            let arrow = if self.sidebar_panels_open[0] { "▼" } else { "▶" };
            lines.push(Line::from(vec![
                Span::styled(format!("{} Context", arrow), section_style),
            ]));
            if self.sidebar_panels_open[0] {
                let tokens_str = if self.context_tokens > 1000 {
                    format!("{}k tokens", self.context_tokens / 1000)
                } else {
                    format!("{} tokens", self.context_tokens)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}", tokens_str), muted),
                ]));
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}% used", self.context_percent), muted),
                ]));
                lines.push(Line::from(vec![
                    Span::styled(format!("  ${:.5} spent", self.session_cost), muted),
                ]));
            }
        }

        // ── Section 1: MCP ──────────────────────────────────
        lines.push(Line::from(""));
        if self.mcp_status.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("▶ MCP", section_style),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  No MCP servers configured", muted),
            ]));
        } else {
            let arrow = if self.sidebar_panels_open[1] { "▼" } else { "▶" };
            let active = self.mcp_status.iter().filter(|(_, s)| s == "connected").count();
            let errs = self.mcp_status.iter().filter(|(_, s)| s == "error").count();
            let summary = if !self.sidebar_panels_open[1] {
                format!(" ({} active{})", active, if errs > 0 { format!(", {} err", errs) } else { String::new() })
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{} MCP{}", arrow, summary), section_style),
            ]));
            if self.sidebar_panels_open[1] {
                for (name, status) in &self.mcp_status {
                    let dot = match status.as_str() {
                        "connected" => green,
                        "error" => red,
                        "needs_auth" => yellow,
                        "disabled" => muted,
                        _ => muted,
                    };
                    let label = match status.as_str() {
                        "connected" => "Connected",
                        "error" => "Error",
                        "needs_auth" => "Needs auth",
                        "disabled" => "Disabled",
                        _ => status,
                    };
                    let display_name: String = name.chars().take(w.saturating_sub(6)).collect();
                    lines.push(Line::from(vec![
                        Span::styled("  •", dot),
                        Span::raw(" "),
                        Span::styled(format!("{} {}", display_name, label), muted),
                    ]));
                }
            }
        }

        // ── Section 2: LSP ──────────────────────────────────
        lines.push(Line::from(""));
        let lsp_arrow = if self.sidebar_panels_open[2] { "▼" } else { "▶" };
        lines.push(Line::from(vec![
            Span::styled(format!("{} LSP", lsp_arrow), section_style),
        ]));
        if self.sidebar_panels_open[2] {
            lines.push(Line::from(vec![
                Span::styled("  LSPs will activate as files are read", muted),
            ]));
        }

        // ── Section 3: Todo ─────────────────────────────────
        lines.push(Line::from(""));
        let todo_arrow = if self.sidebar_panels_open[3] { "▼" } else { "▶" };
        lines.push(Line::from(vec![
            Span::styled(format!("{} Todo", todo_arrow), section_style),
        ]));
        if self.sidebar_panels_open[3] {
            lines.push(Line::from(vec![
                Span::styled("  No active todos", muted),
            ]));
        }

        // ── Section 4: Modified Files ───────────────────────
        lines.push(Line::from(""));
        if self.modified_files.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("▶ Modified Files", section_style),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  No modified files", muted),
            ]));
        } else {
            let arrow = if self.sidebar_panels_open[4] { "▼" } else { "▶" };
            lines.push(Line::from(vec![
                Span::styled(format!("{} Modified Files", arrow), section_style),
            ]));
            if self.sidebar_panels_open[4] {
                for (path, adds, dels) in &self.modified_files {
                    // Truncate path to fit
                    let max_path = w.saturating_sub(8);
                    let display_path: String = if path.len() > max_path {
                        format!("..{}", &path[path.len().saturating_sub(max_path.saturating_sub(2))..])
                    } else {
                        path.clone()
                    };
                    let mut spans: Vec<Span> = Vec::new();
                    spans.push(Span::styled(format!("  {}", display_path), muted));
                    if *adds > 0 {
                        spans.push(Span::raw(" "));
                        spans.push(Span::styled(format!("+{}", adds), green));
                    }
                    if *dels > 0 {
                        spans.push(Span::raw(" "));
                        spans.push(Span::styled(format!("-{}", dels), red));
                    }
                    lines.push(Line::from(spans));
                }
            }
        }

        // ── Footer spacer ───────────────────────────────────
        lines.push(Line::from(""));

        let paragraph = Paragraph::new(lines).style(Style::default().bg(t.background_panel));
        f.render_widget(paragraph, content_area);
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
                    Style::default().fg(t.diff_add).add_modifier(Modifier::DIM)
                } else if line.starts_with('-') && !line.starts_with("---") {
                    Style::default().fg(t.diff_del).add_modifier(Modifier::DIM)
                } else if line.starts_with("@@") || line.starts_with("--- ") || line.starts_with("+++ ") {
                    Style::default().fg(t.diff_hunk).add_modifier(Modifier::DIM)
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

    fn render_toast_overlay(&self, f: &mut Frame, area: Rect, msg: &str) {
        let t = self.theme;
        let text = Span::styled(
            format!(" {} ", msg),
            Style::default()
                .fg(t.success)
                .bg(t.background_panel)
                .add_modifier(Modifier::BOLD),
        );
        let w = msg.len() as u16 + 4;
        let overlay = Rect {
            x: area.right().saturating_sub(w.min(area.width)),
            y: area.bottom().saturating_sub(1),
            width: w.min(area.width),
            height: 1,
        };
        f.render_widget(Clear, overlay);
        f.render_widget(
            ratatui::widgets::Paragraph::new(Line::from(text)),
            overlay,
        );
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
            Style::default().fg(if self.streaming { t.success } else { t.text_muted }),
        );
        let mut spans = vec![left, Span::styled(" │ ", Style::default().fg(t.border)), right];
        if !self.agent_name.is_empty() {
            spans.push(Span::styled(" │ ", Style::default().fg(t.border)));
            spans.push(Span::styled(
                format!(" {} ", self.agent_name),
                Style::default().fg(t.secondary).add_modifier(Modifier::BOLD),
            ));
        }
        if self.plan_mode {
            spans.push(Span::styled(" │ ", Style::default().fg(t.border)));
            spans.push(mode_tag);
        }
        if self.leader_mode {
            spans.push(Span::styled(" │ ", Style::default().fg(t.border)));
            spans.push(Span::styled(
                " LEADER ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ));
        }
        let line = Line::from(spans);
        f.render_widget(ratatui::widgets::Paragraph::new(line).style(Style::default().bg(t.background_element)), area);
    }

    fn render_messages(&self, f: &mut Frame, area: Rect) {
        let t = self.theme;
        let w = area.width as usize;
        let total = self.messages.len();
        // Estimate visible items: assume ~3 lines per message on average
        let max_visible = (area.height.saturating_sub(2) / 3).max(1) as usize;
        let start = if total > max_visible {
            total.saturating_sub(max_visible).saturating_sub(self.scroll)
        } else {
            0
        };
        let items: Vec<ListItem> = self
            .messages
            .iter()
            .enumerate()
            .skip(start)
            .take(max_visible)
            .map(|(idx, m)| {
                let border_color = match m.role.as_str() {
                    "user" => t.user_msg,
                    "assistant" => t.assistant_msg,
                    "reasoning" => t.text_muted,
                    "tool_call" => t.tool_call,
                    "tool_result" => t.tool_result,
                    _ => t.border,
                };

                let bg_color = match m.role.as_str() {
                    "user" => t.background_panel,
                    _ => t.bg,
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

                // Build lines with left border marker "▎" and content
                let bar_style = Style::default().fg(border_color);
                let bar = Span::styled("▎", bar_style);
                let spinner = if self.streaming && m.role == "reasoning" {
                    let spinner_chars = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                    let idx = (self.frame_count / 3) as usize % spinner_chars.len();
                    Span::styled(spinner_chars[idx], Style::default().fg(t.text_muted))
                } else {
                    Span::raw("")
                };
                let pad = Span::raw(" ");

                let mut lines = vec![Line::from(vec![bar, spinner, pad])];

                // Optional timestamp
                if self.show_timestamps {
                    let ts = m.timestamp.format("%H:%M:%S").to_string();
                    lines.push(Line::from(vec![
                        Span::styled("▎", bar_style),
                        Span::raw(" "),
                        Span::styled(ts, Style::default().fg(t.text_muted).add_modifier(Modifier::DIM)),
                    ]));
                }

                let content_width = w.saturating_sub(4);
                if m.role == "assistant" || m.role == "reasoning" {
                    Self::render_highlighted(&display_content, content_width, &mut lines, t);
                } else {
                    let wrapped = textwrap::fill(&display_content, content_width);
                    for l in wrapped.lines() {
                        lines.push(Line::from(vec![
                            Span::styled("▎", bar_style),
                            Span::raw(" "),
                            Span::raw(format!(" {}", l)),
                        ]));
                    }
                }

                ListItem::new(lines).style(Style::default().bg(bg_color))
            })
            .collect();

        let messages = List::new(items)
            .style(Style::default().bg(t.background_panel));

        f.render_widget(messages, area);

        // Scroll indicator when scrolled up from bottom
        if self.scroll > 0 {
            let indicator = format!(
                "  \u{2191} {} more  ",
                self.scroll
            );
            let indicator_len = indicator.len() as u16;
            let indicator_area = Rect {
                x: area.right().saturating_sub(indicator_len),
                y: area.top(),
                width: indicator_len,
                height: 1,
            };
            let indicator_widget = Paragraph::new(Line::from(Span::styled(
                indicator,
                Style::default().fg(t.text_dim).bg(t.background_panel).add_modifier(Modifier::DIM),
            )));
            f.render_widget(indicator_widget, indicator_area);
        }
    }

    fn render_highlighted(content: &str, width: usize, out: &mut Vec<Line>, theme: &Theme) {
        let fence_style = Style::default().fg(theme.border).add_modifier(Modifier::DIM);
        let lang_style = Style::default().fg(theme.tool_call);
        let text_style = Style::default().fg(theme.text);
        let diff_add = Style::default().fg(theme.diff_add).add_modifier(Modifier::DIM);
        let diff_del = Style::default().fg(theme.diff_del).add_modifier(Modifier::DIM);
        let diff_hunk = Style::default().fg(theme.diff_hunk).add_modifier(Modifier::DIM);

        let mut in_code = false;
        let mut code_buf = String::new();
        let mut code_lang = String::new();

        for line in content.lines() {
            if line.starts_with("```") {
                if in_code {
                    if !code_buf.is_empty() {
                        Self::render_code_block(&code_buf, width, &code_lang, out, theme);
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
                    code_lang = lang;
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
            Self::render_code_block(&code_buf, width, &code_lang, out, theme);
        }
    }

    fn render_code_block(code: &str, width: usize, lang: &str, out: &mut Vec<Line>, theme: &Theme) {
        let code_style = Style::default().fg(theme.dim).add_modifier(Modifier::DIM);
        let diff_add = Style::default().fg(theme.diff_add).add_modifier(Modifier::DIM);
        let diff_del = Style::default().fg(theme.diff_del).add_modifier(Modifier::DIM);
        let diff_hunk = Style::default().fg(theme.diff_hunk).add_modifier(Modifier::DIM);

        for line in code.lines() {
            if line.starts_with('+') && !line.starts_with("+++") {
                let wrapped = textwrap::fill(line, width.saturating_sub(2));
                for wl in wrapped.lines() {
                    out.push(Line::from(vec![Span::styled(format!("  {}", wl), diff_add)]));
                }
            } else if line.starts_with('-') && !line.starts_with("---") {
                let wrapped = textwrap::fill(line, width.saturating_sub(2));
                for wl in wrapped.lines() {
                    out.push(Line::from(vec![Span::styled(format!("  {}", wl), diff_del)]));
                }
            } else if line.starts_with("@@") || line.starts_with("--- ") || line.starts_with("+++ ") {
                let wrapped = textwrap::fill(line, width.saturating_sub(2));
                for wl in wrapped.lines() {
                    out.push(Line::from(vec![Span::styled(format!("  {}", wl), diff_hunk)]));
                }
            } else {
                let wrapped = textwrap::fill(line, width.saturating_sub(2));
                for wl in wrapped.lines() {
                    let mut line_spans = vec![Span::raw("  ")];
                    line_spans.extend(Self::syntax_highlight_line(wl, lang, theme));
                    out.push(Line::from(line_spans));
                }
            }
        }
    }

    fn syntax_highlight_line(line: &str, lang: &str, theme: &Theme) -> Vec<Span<'static>> {
        if line.is_empty() {
            return Vec::new();
        }

        let kw_style = Style::default().fg(theme.syntax_keyword);
        let str_style = Style::default().fg(theme.syntax_string);
        let comment_style = Style::default().fg(theme.syntax_comment);
        let num_style = Style::default().fg(theme.syntax_number);
        let builtin_style = Style::default().fg(theme.syntax_builtin);

        let line = line.trim_end();
        let (comment_prefix, is_comment_line) = Self::get_comment_info(line, lang);
        if is_comment_line {
            return vec![Span::styled(line.to_string(), comment_style)];
        }

        let keywords = Self::get_keywords(lang);
        let mut spans = Vec::new();
        let mut i = 0;
        let chars: Vec<char> = line.chars().collect();

        while i < chars.len() {
            // Check for comment (inline)
            if let Some(prefix) = comment_prefix {
                if i + prefix.len() <= chars.len() {
                    let slice: String = chars[i..].iter().collect();
                    if slice.starts_with(prefix) {
                        let rest: String = chars[i..].iter().collect();
                        spans.push(Span::styled(rest, comment_style));
                        break;
                    }
                }
            }

            // Check for string literals
            if chars[i] == '"' {
                let start = i;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' { i += 1; }
                    i += 1;
                }
                if i < chars.len() { i += 1; }
                let s: String = chars[start..i].iter().collect();
                spans.push(Span::styled(s, str_style));
                continue;
            }

            if chars[i] == '\'' {
                let start = i;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    if chars[i] == '\\' { i += 1; }
                    i += 1;
                }
                if i < chars.len() { i += 1; }
                let s: String = chars[start..i].iter().collect();
                spans.push(Span::styled(s, str_style));
                continue;
            }

            if chars[i] == '`' {
                let start = i;
                i += 1;
                while i < chars.len() && chars[i] != '`' {
                    if chars[i] == '\\' { i += 1; }
                    i += 1;
                }
                if i < chars.len() { i += 1; }
                let s: String = chars[start..i].iter().collect();
                spans.push(Span::styled(s, str_style));
                continue;
            }

            // Numbers
            if chars[i].is_ascii_digit() {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == 'x' || chars[i] == 'X' || ('a'..='f').contains(&chars[i]) || ('A'..='F').contains(&chars[i])) {
                    if chars[i] == '.' && i + 1 < chars.len() && !chars[i+1].is_ascii_digit() { break; }
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                spans.push(Span::styled(s, num_style));
                continue;
            }

            // Identifiers and keywords
            if chars[i].is_alphabetic() || chars[i] == '_' {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();

                if keywords.contains(&word.as_str()) {
                    spans.push(Span::styled(word, kw_style));
                } else if Self::is_builtin(&word, lang) {
                    spans.push(Span::styled(word, builtin_style));
                } else {
                    spans.push(Span::raw(word));
                }
                continue;
            }

            // Skip whitespace and other characters
            spans.push(Span::raw(chars[i].to_string()));
            i += 1;
        }

        spans
    }
    fn get_comment_info(line: &str, lang: &str) -> (Option<&'static str>, bool) {
        let lang = crate::util::filetype::normalize_language(lang);
        let (single, can_be_inline): (&str, bool) = match lang {
            "rust" | "go" | "c" | "cpp" | "java" | "javascript" | "typescript" | "swift" | "kotlin" | "scala" | "dart" | "zig" => ("//", true),
            "python" | "r" | "ruby" | "yaml" | "toml" | "ini" | "cfg" | "perl" | "elixir" => ("#", true),
            "lua" => ("--", true),
            "sql" => ("--", true),
            "haskell" => ("--", true),
            "clojure" | "lisp" | "scheme" => (";", true),
            "html" | "xml" | "svg" => ("<!--", false),
            "php" => ("//", true),
            "bash" | "shell" | "sh" => ("#", true),
            _ => ("", false),
        };

        if single.is_empty() {
            return (None, false);
        }

        let trimmed = line.trim_start();
        if trimmed.starts_with(single) {
            return (Some(single), true);
        }
        if can_be_inline {
            return (Some(single), false);
        }

        (None, false)
    }

    fn get_keywords(lang: &str) -> &'static [&'static str] {
        let lang = crate::util::filetype::normalize_language(lang);
        match lang {
        "rust" => &["as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "try"],
        "go" => &["break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var"],
        "python" => &["False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield"],
        "javascript" | "typescript" => &["async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "enum", "export", "extends", "false", "finally", "for", "function", "if", "import", "in", "instanceof", "let", "new", "null", "of", "return", "super", "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while", "with", "yield"],
        "java" => &["abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "const", "continue", "default", "do", "double", "else", "enum", "extends", "false", "final", "finally", "float", "for", "goto", "if", "implements", "import", "instanceof", "int", "interface", "long", "native", "new", "null", "package", "private", "protected", "public", "return", "short", "static", "strictfp", "super", "switch", "synchronized", "this", "throw", "throws", "transient", "true", "try", "void", "volatile", "while"],
        "c" | "cpp" => &["auto", "bool", "break", "case", "catch", "char", "class", "const", "constexpr", "continue", "default", "delete", "do", "double", "else", "enum", "explicit", "extern", "false", "float", "for", "friend", "goto", "if", "inline", "int", "long", "namespace", "new", "noexcept", "nullptr", "operator", "override", "private", "protected", "public", "return", "short", "signed", "sizeof", "static", "struct", "switch", "template", "this", "throw", "true", "try", "typedef", "typename", "union", "unsigned", "using", "virtual", "void", "volatile", "while"],
        "ruby" => &["BEGIN", "END", "alias", "and", "begin", "break", "case", "class", "def", "defined?", "do", "else", "elsif", "end", "ensure", "false", "for", "if", "in", "module", "next", "nil", "not", "or", "redo", "rescue", "retry", "return", "self", "super", "then", "true", "undef", "unless", "until", "when", "while", "yield"],
        "php" => &["__CLASS__", "__DIR__", "__FILE__", "__FUNCTION__", "__LINE__", "__METHOD__", "__NAMESPACE__", "__TRAIT__", "abstract", "and", "array", "as", "break", "callable", "case", "catch", "class", "clone", "const", "continue", "declare", "default", "die", "do", "echo", "else", "elseif", "empty", "enddeclare", "endfor", "endforeach", "endif", "endswitch", "endwhile", "eval", "exit", "extends", "false", "final", "finally", "fn", "for", "foreach", "function", "global", "goto", "if", "implements", "include", "include_once", "instanceof", "insteadof", "interface", "isset", "list", "match", "namespace", "new", "null", "or", "print", "private", "protected", "public", "readonly", "require", "require_once", "return", "static", "switch", "throw", "trait", "true", "try", "unset", "use", "var", "while", "xor", "yield"],
        "swift" => &["Protocol", "Self", "Type", "actor", "any", "associatedtype", "async", "await", "break", "case", "catch", "class", "continue", "convenience", "default", "defer", "deinit", "didSet", "do", "dynamic", "else", "enum", "extension", "fallthrough", "false", "fileprivate", "for", "func", "get", "guard", "if", "import", "in", "indirect", "infix", "init", "inout", "internal", "is", "lazy", "let", "macro", "mutating", "nil", "nonmutating", "open", "operator", "optional", "override", "package", "postfix", "precedence", "prefix", "private", "protocol", "public", "repeat", "required", "rethrows", "return", "self", "set", "some", "static", "struct", "subscript", "super", "switch", "throw", "throws", "true", "try", "typealias", "unowned", "var", "weak", "where", "while", "willSet"],
        "kotlin" => &["actual", "annotation", "as", "as?", "break", "by", "catch", "class", "companion", "const", "constructor", "continue", "crossinline", "data", "delegate", "do", "dynamic", "else", "enum", "expect", "external", "false", "field", "file", "final", "finally", "for", "fun", "if", "import", "in", "!in", "infix", "init", "inline", "inner", "interface", "internal", "is", "!is", "lateinit", "noinline", "null", "object", "open", "operator", "out", "override", "package", "param", "private", "property", "protected", "public", "receiver", "reified", "return", "sealed", "set", "setparam", "super", "suspend", "tailrec", "this", "throw", "true", "try", "typealias", "typeof", "val", "var", "vararg", "when", "where", "while"],
        "scala" => &["abstract", "case", "catch", "class", "def", "do", "else", "enum", "extends", "false", "final", "finally", "for", "forSome", "given", "if", "implicit", "import", "lazy", "match", "new", "null", "object", "override", "package", "private", "protected", "public", "return", "sealed", "self", "super", "then", "throw", "trait", "true", "try", "type", "using", "val", "var", "while", "with", "yield"],
        "lua" => &["and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"],
        "haskell" => &["class", "data", "default", "deriving", "do", "else", "family", "forall", "foreign", "hiding", "if", "import", "in", "infix", "infixl", "infixr", "instance", "let", "module", "newtype", "of", "open", "pattern", "qualified", "then", "type", "where", "_"],
        "dart" => &["abstract", "as", "assert", "async", "await", "break", "case", "catch", "class", "const", "continue", "covariant", "default", "deferred", "do", "dynamic", "else", "enum", "export", "extends", "extension", "external", "factory", "false", "final", "finally", "for", "Function", "get", "hide", "if", "implements", "import", "in", "interface", "is", "late", "library", "mixin", "new", "null", "on", "operator", "optional", "part", "required", "rethrow", "return", "set", "show", "static", "super", "switch", "sync", "this", "throw", "true", "try", "typedef", "var", "void", "while", "with", "yield"],
        "r" => &["FALSE", "NULL", "NA", "NaN", "TRUE", "break", "else", "for", "function", "if", "in", "Inf", "next", "repeat", "return", "while"],
        _ => &[],
        }
    }

    fn is_builtin(word: &str, lang: &str) -> bool {
        let lang = crate::util::filetype::normalize_language(lang);
        match lang {
        "python" => matches!(word, "print" | "len" | "range" | "type" | "str" | "int" | "float" | "list" | "dict" | "set" | "tuple" | "bool" | "super" | "self" | "open" | "map" | "filter" | "zip" | "enumerate" | "sorted" | "reversed" | "any" | "all" | "sum" | "min" | "max" | "abs" | "round" | "isinstance" | "hasattr" | "getattr" | "setattr" | "ValueError" | "TypeError" | "KeyError" | "Exception" | "BaseException" | "object" | "property" | "staticmethod" | "classmethod"),
        "javascript" | "typescript" => matches!(word, "console" | "log" | "error" | "warn" | "require" | "module" | "exports" | "process" | "Buffer" | "setTimeout" | "setInterval" | "fetch" | "Promise" | "Array" | "Object" | "String" | "Number" | "Boolean" | "Map" | "Set" | "Symbol" | "JSON" | "Math" | "Date" | "RegExp" | "Error" | "undefined" | "null" | "true" | "false" | "window" | "document" | "globalThis" | "exports" | "describe" | "it" | "test" | "expect" | "jest"),
        "ruby" => matches!(word, "puts" | "print" | "p" | "require" | "include" | "extend" | "attr_accessor" | "attr_reader" | "attr_writer" | "private" | "protected" | "public" | "raise" | "fail" | "catch" | "throw" | "lambda" | "proc" | "eval" | "loop" | "sleep" | "gets" | "chomp" | "inspect" | "to_s" | "to_i" | "to_f" | "nil?" | "empty?" | "length" | "size" | "each" | "map" | "select" | "reject" | "reduce" | "inject" | "sort" | "uniq" | "first" | "last"),
        "php" => matches!(word, "echo" | "print" | "die" | "exit" | "isset" | "unset" | "empty" | "require" | "require_once" | "include" | "include_once" | "defined" | "array" | "count" | "strlen" | "strpos" | "substr" | "explode" | "implode" | "json_encode" | "json_decode" | "preg_match" | "sprintf" | "var_dump" | "error_log" | "header" | "session_start" | "setcookie" | "is_null" | "is_numeric" | "PHP_EOL" | "true" | "false" | "null"),
        _ => false,
        }
    }

    fn render_autocomplete_popup(&self, f: &mut Frame, area: Rect, candidates: &[String], idx: isize) {
        let t = self.theme;
        let is_slash = self.input.starts_with('/') && !self.input.contains(' ');
        let header = if is_slash { " Commands " } else { " @ Files " };

        let items: Vec<Line> = candidates
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let (icon, name) = if let Some(r) = c.strip_prefix("ref:") {
                    (" ~ ", r)
                } else if is_slash {
                    (" / ", c.as_str())
                } else {
                    (" > ", c.as_str())
                };
                let selected = i as isize == idx;
                let style = if selected {
                    Style::default().fg(t.text).bg(t.accent)
                } else {
                    Style::default().fg(t.text).bg(t.background_panel)
                };
                let dim = if selected { Modifier::empty() } else { Modifier::DIM };
                Line::from(vec![
                    Span::styled(icon, style.add_modifier(dim)),
                    Span::styled(name, style.add_modifier(dim)),
                ])
            })
            .collect();

        let border_style = Style::default().fg(t.border_active);
        let block = Block::default()
            .title(header)
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(Style::default().bg(t.background_panel));

        let list = Paragraph::new(items).block(block);
        f.render_widget(list, area);
    }

    fn render_input(&self, f: &mut Frame, area: Rect) {
        let t = self.theme;
        let border_color = if self.leader_mode { t.border_active } else { t.border };

        let outer_block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.background_element));

        let inner = outer_block.inner(area);

        // Split inner area into input text + status row at bottom
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        f.render_widget(outer_block, area);

        let input = Paragraph::new(self.input.as_str())
            .style(Style::default().fg(t.text).bg(t.background_element))
            .wrap(Wrap { trim: true });

        f.render_widget(input, chunks[0]);

        let cursor_pos = self.input.len() as u16;
        f.set_cursor_position((chunks[0].x + cursor_pos + 1, chunks[0].y + 1));

        // Render status inside the input box
        self.render_status(f, chunks[1]);
    }

    // ── Dialog rendering ────────────────────────────────────

    fn render_dialog(&self, f: &mut Frame) {
        let dialog = match &self.dialog {
            Some(d) => d,
            None => return,
        };
        let t = self.theme;
        let area = f.area();

        // Clear area for overlay effect
        f.render_widget(Clear, area);

        use ActiveDialog::*;
        match dialog {
            Help => self.render_help_dialog(f, t),
            Alert { title, message } => self.render_alert_dialog(f, t, title, message),
            Workspace => self.render_workspace_dialog(f, t),
            Confirm { title, message, action } => self.render_confirm_dialog(f, t, title, message, action),
            Agent { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let filtered: Vec<&SelectOption> = filtered.iter().collect();
                Self::render_select_dialog(f, t, &filtered, *selected, filter, "Select Agent", area);
            }
            Model { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let filtered: Vec<&SelectOption> = filtered.iter().collect();
                Self::render_select_dialog(f, t, &filtered, *selected, filter, "Select Model", area);
            }
            Theme { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let filtered: Vec<&SelectOption> = filtered.iter().collect();
                Self::render_select_dialog(f, t, &filtered, *selected, filter, "Select Theme", area);
            }
            SessionList { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let filtered: Vec<&SelectOption> = filtered.iter().collect();
                Self::render_select_dialog(f, t, &filtered, *selected, filter, "Saved Sessions", area);
            }
            MCPStatus { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let filtered: Vec<&SelectOption> = filtered.iter().collect();
                Self::render_select_dialog(f, t, &filtered, *selected, filter, "MCP Tools", area);
            }
            Stash { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let filtered: Vec<&SelectOption> = filtered.iter().collect();
                Self::render_select_dialog(f, t, &filtered, *selected, filter, "Stashed Prompts", area);
            }
            Skill { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let filtered: Vec<&SelectOption> = filtered.iter().collect();
                Self::render_select_dialog(f, t, &filtered, *selected, filter, "Select Skill", area);
            }
            Status { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let filtered: Vec<&SelectOption> = filtered.iter().collect();
                Self::render_select_dialog(f, t, &filtered, *selected, filter, "Session Status", area);
            }
            CommandPalette { options, selected, filter } => {
                let filtered = Self::filter_options(options, filter);
                let filtered: Vec<&SelectOption> = filtered.iter().collect();
                Self::render_select_dialog(f, t, &filtered, *selected, filter, "Command Palette", area);
            }
            Prompt { title, value, action: _, cursor } => {
                Self::render_prompt_dialog(f, t, title, value, *cursor, area);
            }
        }
    }

    fn render_prompt_dialog(f: &mut Frame, t: &Theme, title: &str, value: &str, cursor: usize, area: Rect) {
        let max_w = 60.min(area.width.saturating_sub(4)) as usize;
        let inner = Self::centered_rect(area, max_w.min(60) as u16, 5);

        let display: String = value.chars().take(max_w.saturating_sub(4)).collect();
        let cursor_pos = cursor.min(display.len());

        let mut spans = Vec::new();
        if cursor_pos > 0 {
            spans.push(Span::styled(display[..cursor_pos].to_string(), Style::default().fg(t.text)));
        }
        spans.push(Span::styled("█", Style::default().fg(t.primary)));
        if cursor_pos < display.len() {
            spans.push(Span::styled(display[cursor_pos..].to_string(), Style::default().fg(t.text)));
        }

        let input_line = Line::from(spans);
        let lines = vec![
            Line::from(Span::styled(
                format!(" {}", title),
                Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            input_line,
            Line::from(""),
            Line::from(Span::styled(
                "  Enter confirm  Esc cancel",
                Style::default().fg(t.text_dim),
            )),
        ];

        let para = Paragraph::new(lines)
            .style(Style::default().bg(t.background_panel))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(t.primary)));
        f.render_widget(Clear, inner);
        f.render_widget(para, inner);
    }

    fn render_help_dialog(&self, f: &mut Frame, t: &Theme) {
        let area = Self::dialog_area(f.area());
        let help_text = vec![
            "OpenCode TUI Key Bindings",
            "",
            "General:",
            "  Ctrl+C / q     Quit",
            "  Space          Leader menu (empty input)",
            "  Esc            Cancel streaming / Close dialogs",
            "  Ctrl+Y         Copy last response",
            "  Ctrl+E         Open last edited file in $EDITOR",
            "  Ctrl+R         Toggle reasoning visibility",
            "  Ctrl+T         Toggle timestamps",
            "  Ctrl+B         Toggle sidebar",
            "  Ctrl+P         Command palette",
            "  Ctrl+O         Toggle tool output collapse",
            "  Tab/Shift+Tab    Navigate autocomplete",
            "  Enter          Submit / select autocomplete",
            "",
            "Leader keys:",
            "  b  Toggle sidebar",
            "  k  Command palette",
            "  w  Workspace info",
            "  r  Rename session",
            "  f  Insert /diagnostics",
            "  s  Session list (dialog)",
            "  /  Plan mode",
            "  t  Theme picker (dialog)",
            "  n  New session",
            "  h  Help dialog",
            "  d  Diff viewer",
            "  e  Open last edited file",
            "  m  Model picker (dialog)",
            "  a  Agent picker (dialog)",
            "  q  Quit",
            "",
            "Dialog navigation:",
            "  ↑/k  Previous item    ↓/j  Next item",
            "  Enter  Select item    Esc   Close",
            "  Type to filter results",
            "",
            "Press any key to close.",
        ];
        let text: Vec<Line> = help_text.iter().map(|line| {
            let style = if line.starts_with("OpenCode") {
                Style::default().fg(t.primary).add_modifier(Modifier::BOLD)
            } else if line.ends_with(':') {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text)
            };
            Line::from(Span::styled(*line, style))
        }).collect();

        let para = Paragraph::new(text)
            .style(Style::default().bg(t.background_panel))
            .block(Block::default().borders(Borders::ALL).title(" Help ").border_style(Style::default().fg(t.primary)));
        let inner = Self::centered_rect(area, 60, help_text.len() as u16 + 4);
        f.render_widget(Clear, inner);
        f.render_widget(para, inner);
    }

    fn render_alert_dialog(&self, f: &mut Frame, t: &Theme, title: &str, message: &str) {
        let area = Self::dialog_area(f.area());
        let lines: usize = message.lines().count();
        let text = Paragraph::new(message.to_string())
            .style(Style::default().bg(t.background_panel))
            .block(Block::default().borders(Borders::ALL)
                .title(format!(" {} ", title))
                .border_style(Style::default().fg(t.primary)));
        let inner = Self::centered_rect(area, 60, lines as u16 + 4);
        f.render_widget(Clear, inner);
        f.render_widget(text, inner);
    }

    fn render_confirm_dialog(&self, f: &mut Frame, t: &Theme, title: &str, message: &str, _action: &str) {
        let area = Self::dialog_area(f.area());
        let lines: Vec<Line> = vec![
            Line::from(Span::styled(message.to_string(), Style::default().fg(t.text))),
            Line::from(""),
            Line::from(Span::styled("  (y)es / (n)o  ", Style::default().fg(t.warning).add_modifier(Modifier::BOLD))),
        ];
        let para = Paragraph::new(lines)
            .style(Style::default().bg(t.background_panel))
            .block(Block::default().borders(Borders::ALL)
                .title(format!(" {} ", title))
                .border_style(Style::default().fg(t.warning)));
        let inner = Self::centered_rect(area, 50, 6);
        f.render_widget(Clear, inner);
        f.render_widget(para, inner);
    }

    fn render_workspace_dialog(&self, f: &mut Frame, t: &Theme) {
        let area = Self::dialog_area(f.area());
        let mut lines = Vec::new();
        if let Ok(session) = self.session.try_lock() {
            lines.push(Line::from(Span::styled(format!("Directory: {}", session.cwd), Style::default().fg(t.text))));
            lines.push(Line::from(Span::styled(format!("Model: {}", session.model), Style::default().fg(t.text))));
            lines.push(Line::from(Span::styled(
                format!("Tools: {} available", session.tools.len()),
                Style::default().fg(t.text),
            )));
            lines.push(Line::from(Span::styled(
                format!("Messages: {} in session", session.messages.len()),
                Style::default().fg(t.text),
            )));
            lines.push(Line::from(Span::styled(
                format!("Stats: {} prompts, {} total tokens", session.stats.prompt_count, session.stats.total_tokens),
                Style::default().fg(t.text_dim),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Config providers: {}", session.config.provider.len()),
                Style::default().fg(t.text_dim),
            )));
            lines.push(Line::from(Span::styled(
                format!("Config MCP servers: {}", session.config.mcp.len()),
                Style::default().fg(t.text_dim),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press any key to close  ",
            Style::default().fg(t.text_dim).add_modifier(Modifier::DIM),
        )));
        let para = Paragraph::new(lines)
            .style(Style::default().bg(t.background_panel))
            .block(Block::default().borders(Borders::ALL)
                .title(" Workspace Info ")
                .border_style(Style::default().fg(t.primary)));
        let inner = Self::centered_rect(area, 60, 12);
        f.render_widget(Clear, inner);
        f.render_widget(para, inner);
    }

    fn render_select_dialog<'a>(
        f: &mut Frame,
        t: &Theme,
        options: &[&'a SelectOption],
        selected: usize,
        filter: &str,
        title: &str,
        area: Rect,
    ) {
        let max_visible = 20usize;
        let scroll = if selected >= max_visible { selected - max_visible + 1 } else { 0 };
        let total_str = options.len().to_string();

        let mut lines: Vec<Line> = Vec::new();

        // Filter indicator row
        let filter_display = if !filter.is_empty() {
            format!(" filter: {}", filter)
        } else {
            " type to filter".to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(filter_display, Style::default().fg(t.text_dim)),
            Span::styled(
                format!("  {} items", total_str),
                Style::default().fg(t.text_dim).add_modifier(Modifier::DIM),
            ),
        ]));
        lines.push(Line::from(""));

        // Group items by category
        struct Group<'a> {
            name: &'a str,
            items: Vec<(usize, &'a SelectOption)>,
        }
        let mut groups: Vec<Group> = Vec::new();
        let mut current_cat: Option<&str> = None;
        for (i, opt) in options.iter().enumerate().skip(scroll).take(max_visible) {
            let cat = opt.category.as_deref().unwrap_or("");
            if current_cat != Some(cat) {
                current_cat = Some(cat);
                groups.push(Group { name: cat, items: Vec::new() });
            }
            if let Some(g) = groups.last_mut() {
                g.items.push((i, opt));
            }
        }

        for group in &groups {
            // Category header
            if !group.name.is_empty() {
                let sep = "─".repeat(50);
                lines.push(Line::from(vec![
                    Span::styled(format!(" {}", group.name), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
                    Span::styled(sep, Style::default().fg(t.border).add_modifier(Modifier::DIM)),
                ]));
            }

            // Items in this group
            for &(i, opt) in &group.items {
                let is_sel = i == selected;
                let style = if is_sel {
                    Style::default().fg(t.bg).bg(t.primary)
                } else {
                    Style::default().fg(t.text)
                };
                let prefix = if is_sel { " ▸ " } else { "   " };
                let desc = opt.description.as_deref().map(|d| format!("  {}", d)).unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled(prefix, if is_sel { Style::default().fg(t.primary) } else { Style::default().fg(t.text_dim) }),
                    Span::styled(opt.title.to_string(), if is_sel { style } else { Style::default().fg(t.text) }),
                    Span::styled(desc, Style::default().fg(t.text_dim).add_modifier(Modifier::DIM)),
                ]));
            }
        }

        // Footer
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  \u{2191}\u{2193} navigate", Style::default().fg(t.text_dim).add_modifier(Modifier::DIM)),
            Span::styled("  Enter select", Style::default().fg(t.text_dim).add_modifier(Modifier::DIM)),
            Span::styled("  Esc close", Style::default().fg(t.text_dim).add_modifier(Modifier::DIM)),
        ]));

        let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
        let dialog_area = Self::centered_rect(area, 70, height);
        let para = Paragraph::new(lines)
            .style(Style::default().bg(t.background_panel))
            .block(Block::default().borders(Borders::ALL)
                .title(format!(" {} ", title))
                .border_style(Style::default().fg(t.primary)));
        f.render_widget(Clear, dialog_area);
        f.render_widget(para, dialog_area);
    }

    fn dialog_area(area: Rect) -> Rect {
        let width = area.width.min(80);
        let height = area.height.min(40);
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 3;
        Rect { x, y, width, height }
    }

    fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 3;
        Rect { x, y, width: width.min(area.width), height: height.min(area.height) }
    }
}

fn send_notification(summary: &str, body: &str) {
    if let Err(_) = Notification::new()
        .summary(summary)
        .body(body)
        .timeout(3000)
        .show()
    {
        let _ = print!("\x07");
        let _ = io::Write::flush(&mut io::stdout());
    }
}
