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
use ratatui::style::{Color, Modifier, Style};
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
    pub scroll_accel: u32,
    pub scroll_speed: usize,
    pub diff_style: String,
    pub diff_source: String,  // "working_tree" or "last_turn"
    pub diff_wrap_mode: String,  // "none", "word", or "char"
    pub reviewed_files: std::collections::HashSet<String>,
    pub quit: bool,
    pub stream_rx: Option<mpsc::Receiver<StreamEvent>>,
    pub pending_response: String,
    pub streaming: bool,
    pub thinking_started: Option<chrono::DateTime<chrono::Utc>>,
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
    pub autocomplete_frecency: std::collections::HashMap<String, u32>,
    pub theme: Theme,
    pub theme_name: String,
    pub reasoning: String,
    pub reasoning_visible: bool,
    pub collapsed: std::collections::HashSet<usize>,
    pub toast: Option<(String, u8, Color)>,
    pub show_timestamps: bool,
    pub leader_mode: bool,
    pub file_watcher_rx: Option<std_mpsc::Receiver<String>>,
    pub diff_viewer: Option<(Vec<String>, usize, String)>,  // (lines, scroll_offset, style)
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
    pub show_splash: bool,
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

// ── Layout constants ────────────────────────────────────
const SIDEBAR_WIDTH: u16 = 36;
const DIALOG_WIDTH: u16 = 60;
const DIALOG_HEIGHT: u16 = 40;
const TOAST_DURATION_NORMAL: u8 = 30;
const TOAST_DURATION_ERROR: u8 = 60;
const TOAST_DURATION_LONG: u8 = 80;
const TOOL_RESULT_COLLAPSE_THRESHOLD: usize = 200;

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
        let scroll_speed = session.config.scroll_speed.unwrap_or(10);
        let diff_style = session.config.diff_style.clone().unwrap_or_else(|| "unified".to_string());
        let diff_source = "working_tree".to_string();
        let diff_wrap_mode = session.config.diff_wrap_mode.clone().unwrap_or_else(|| "none".to_string());
        Self {
            session: Arc::new(Mutex::new(session)),
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            input_history: Vec::new(),
            history_index: -1,
            saved_input: String::new(),
            scroll: 0,
            scroll_accel: 1,
            scroll_speed,
            diff_style,
            diff_source,
            diff_wrap_mode,
            reviewed_files: std::collections::HashSet::new(),
            quit: false,
            stream_rx: None,
            pending_response: String::new(),
            streaming: false,
            thinking_started: None,
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
            autocomplete_frecency: std::collections::HashMap::new(),
            theme: crate::theme::Theme { ..crate::theme::DEFAULT },
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
            show_splash: true,
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

        // Print session epilogue
        if let Ok(session) = self.session.try_lock() {
            let stats = &session.stats;
            let epilogue = format!(
                "\n\
                 ═══════════════════════════════════════\n\
                  Session Summary\n\
                 ═══════════════════════════════════════\n\
                  Session ID    │ {}\n\
                  Model         │ {}\n\
                  Prompts       │ {}\n\
                  Tool calls    │ {}\n\
                  Files changed │ {}\n\
                  Prompt tokens │ {}\n\
                  Output tokens  │ {}\n\
                  Total tokens  │ {}\n\
                  Session cost  │ ${:.5}\n\
                 ═══════════════════════════════════════\n",
                &session.id[..session.id.len().min(8)],
                session.model,
                stats.prompt_count,
                stats.tool_call_count,
                self.modified_files.len(),
                stats.prompt_tokens,
                stats.completion_tokens,
                stats.total_tokens,
                self.session_cost,
            );
            println!("{}", epilogue);
        }

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
        let prev_len = self.messages.len();
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
                        let args_summary = crate::util::tool_display::format_tool_args(&name, &arguments);
                        self.context_tokens += arguments.to_string().len() / 4;
                        self.context_percent = ((self.context_tokens as f64 / 100000.0) * 100.0) as u8;
                        let icon = crate::util::tool_display::tool_icon(&name);
                        let hname = crate::util::tool_display::human_name(&name);
                        let content = if args_summary.is_empty() {
                            format!("{} {} ({})", icon, hname, short)
                        } else {
                            format!("{} {} ({})\n{}", icon, hname, short, args_summary)
                        };
                        self.messages.push(TuiMessage {
            age: 0, timestamp: chrono::Utc::now(),
                            role: "tool_call".to_string(),
                            content,
                        });
                    }
                    StreamEvent::PermissionRequest { request_id, tool_name, args } => {
                        let args_summary = crate::util::tool_display::format_tool_args(&tool_name, &args);
                        let icon = crate::util::tool_display::tool_icon(&tool_name);
                        let hname = crate::util::tool_display::human_name(&tool_name);
                        let content = if args_summary.is_empty() {
                            format!("{} {} (AWAITING APPROVAL)", icon, hname)
                        } else {
                            format!("{} {} (AWAITING APPROVAL)\n{}", icon, hname, args_summary)
                        };
                        self.messages.push(TuiMessage {
            age: 0, timestamp: chrono::Utc::now(),
                            role: "tool_call".to_string(),
                            content,
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
                        let is_long = content.len() > TOOL_RESULT_COLLAPSE_THRESHOLD;
                        self.messages.push(TuiMessage {
            age: 0, timestamp: chrono::Utc::now(),
                            role: "tool_result".to_string(),
                            content,
                        });
                        if is_long {
                            self.collapsed.insert(self.messages.len() - 1);
                        }
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
                                self.toast = Some((format!("Auto-compacted: removed {} messages", removed), TOAST_DURATION_LONG, Color::Rgb(0xe9, 0xab, 0x2f)));
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
            self.thinking_started = None;
            self.stream_rx = None;
            self.save_session();
        }
        // Sticky scroll: when scrolled up, keep view stable as new messages arrive
        let new_len = self.messages.len();
        let delta = new_len.saturating_sub(prev_len);
        if self.scroll > 0 && delta > 0 {
            self.scroll = self.scroll.saturating_add(delta);
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
                    Err(e) => self.show_error_toast(format!("Delete failed: {}", e)),
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
                        self.show_error_toast(format!("Rename failed: {}", e));
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
                self.show_success_toast("Copied last response to clipboard".to_string());
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
                        let diff = if self.diff_source == "last_turn" {
                            s.show_last_turn_diff()
                        } else {
                            s.show_diff()
                        };
                        if diff.starts_with("---") {
                            let lines: Vec<String> = diff.lines().map(|l| l.to_string()).collect();
                            self.diff_viewer = Some((lines, 0, self.diff_style.clone()));
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
        self.scroll = 0;
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
                self.show_warning_toast("No edited file to open".to_string());
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
                let lower_q = file_query.to_lowercase();

                // File candidates via fd
                let mut candidates: Vec<String> = Vec::new();
                if !file_query.starts_with("ref:") {
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
                        let frecency = &self.autocomplete_frecency;
                        file_candidates.sort_by_cached_key(|c| {
                            let lower_c = c.to_lowercase();
                            (
                                !lower_c.starts_with(&lower_q),
                                lower_c.find(&lower_q).unwrap_or(usize::MAX),
                                std::cmp::Reverse(frecency.get(c).copied().unwrap_or(0)),
                            )
                        });
                    }
                    for c in &mut file_candidates {
                        let path = std::path::Path::new(c);
                        if path.is_dir() {
                            c.push('/');
                        }
                    }
                    candidates.extend(file_candidates);
                }

                // MCP tool candidates
                let mcp_candidates: Vec<String> = self.mcp_status.iter()
                    .filter(|(_, status)| status == "connected")
                    .filter(|(name, _)| name.to_lowercase().contains(&lower_q))
                    .map(|(name, _)| format!("mcp:{}", name))
                    .collect();
                candidates.extend(mcp_candidates);

                // Reference candidates
                let ref_candidates: Vec<String> = self.references.iter()
                    .filter(|r| r.name.to_lowercase().contains(&lower_q))
                    .map(|r| format!("ref:{}", r.name))
                    .collect();
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
            *self.autocomplete_frecency.entry(selected.clone()).or_insert(0) += 1;
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
        *self.autocomplete_frecency.entry(selected.clone()).or_insert(0) += 1;
        self.autocomplete_candidates.clear();
        self.autocomplete_idx = -1;
        true
    }

    async fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        // Dismiss splash on first key press
        if self.show_splash {
            self.show_splash = false;
        }

        // Dialog mode handles all keys when active
        if self.dialog.is_some() {
            self.handle_dialog_key(key)?;
            return Ok(());
        }

        // Diff viewer mode handles all keys when active
        if self.diff_viewer.is_some() {
            let lines_len = self.diff_viewer.as_ref().map(|v| v.0.len()).unwrap_or(0);
            let scroll = self.diff_viewer.as_ref().map(|v| v.1).unwrap_or(0);

            // Find hunk positions for [ / ] navigation
            let hunk_positions: Vec<usize> = self.diff_viewer.as_ref()
                .map(|v| v.0.iter().enumerate()
                    .filter(|(_, l)| l.starts_with("@@"))
                    .map(|(i, _)| i)
                    .collect())
                .unwrap_or_default();

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
                KeyCode::Char('[') | KeyCode::Char('{') => {
                    // Jump to previous hunk
                    if let Some(&pos) = hunk_positions.iter().rev().find(|&&p| p < scroll) {
                        self.diff_viewer.as_mut().map(|v| v.1 = pos);
                    }
                }
                KeyCode::Char(']') | KeyCode::Char('}') => {
                    // Jump to next hunk
                    if let Some(&pos) = hunk_positions.iter().find(|&&p| p > scroll) {
                        self.diff_viewer.as_mut().map(|v| v.1 = pos);
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('p') => {
                    if let Some((ref lines, ref mut scroll, _)) = self.diff_viewer {
                        let file_positions: Vec<usize> = lines.iter().enumerate()
                            .filter(|(_, l)| l.starts_with("--- a/"))
                            .map(|(i, _)| i)
                            .collect();
                        if key.code == KeyCode::Char('n') {
                            if let Some(&pos) = file_positions.iter().find(|&&p| p > *scroll) {
                                *scroll = pos;
                            }
                        } else if let Some(&pos) = file_positions.iter().rev().find(|&&p| p < *scroll) {
                            *scroll = pos;
                        }
                    }
                }
                KeyCode::Char('v') => {
                    // Toggle diff style
                    if let Some(ref mut viewer) = self.diff_viewer {
                        viewer.2 = if viewer.2 == "unified" {
                            "side_by_side".to_string()
                        } else {
                            "unified".to_string()
                        };
                    }
                }
                KeyCode::Char('s') => {
                    // Toggle diff source (working_tree / last_turn)
                    self.diff_source = if self.diff_source == "working_tree" {
                        "last_turn".to_string()
                    } else {
                        "working_tree".to_string()
                    };
                    // Rebuild diff with new source
                    if let Ok(s) = self.session.try_lock() {
                        let diff = if self.diff_source == "last_turn" {
                            s.show_last_turn_diff()
                        } else {
                            s.show_diff()
                        };
                        if diff.starts_with("---") {
                            let lines: Vec<String> = diff.lines().map(|l| l.to_string()).collect();
                            self.diff_viewer = Some((lines, 0, self.diff_style.clone()));
                        }
                    }
                }
                KeyCode::Char('m') => {
                    // Toggle review mark on current file
                    if let Some((ref lines, ref scroll, _)) = self.diff_viewer {
                        let file_positions: Vec<&str> = lines.iter()
                            .filter(|l| l.starts_with("--- a/"))
                            .filter_map(|l| l.strip_prefix("--- a/"))
                            .collect();
                        let current = file_positions.iter().rev()
                            .find(|path| {
                                let pos = lines.iter().position(|l| l.starts_with("--- a/") && l.contains(**path));
                                pos.map_or(false, |p| p <= *scroll)
                            });
                        if let Some(file_path) = current {
                            let path = file_path.to_string();
                            if self.reviewed_files.contains(&path) {
                                self.reviewed_files.remove(&path);
                            } else {
                                self.reviewed_files.insert(path);
                            }
                        }
                    }
                }
                KeyCode::Char('w') => {
                    // Cycle wrap mode
                    self.diff_wrap_mode = match self.diff_wrap_mode.as_str() {
                        "none" => "word".to_string(),
                        "word" => "char".to_string(),
                        _ => "none".to_string(),
                    };
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
                _ => { self.show_warning_toast("Unknown leader key".to_string()); }
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
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
                self.show_success_toast("Copied last response to clipboard".to_string());
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
                    self.thinking_started = Some(chrono::Utc::now());
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
                let step = (self.scroll_speed * self.scroll_accel as usize).min(self.scroll_speed * 8);
                self.scroll = self.scroll.saturating_add(step);
                self.scroll_accel = (self.scroll_accel + 1).min(8);
            }
            KeyCode::PageDown => {
                let step = (self.scroll_speed * self.scroll_accel as usize).min(self.scroll_speed * 8);
                self.scroll = self.scroll.saturating_sub(step);
                self.scroll_accel = (self.scroll_accel + 1).min(8);
            }
            _ => {
                self.scroll_accel = 1;
            }
        }
        Ok(())
    }

    fn toggle_collapse_last_tool(&mut self) {
        let tool_indices: Vec<usize> = self.messages.iter().enumerate()
            .filter(|(_, m)| m.role == "tool_result" || m.role == "tool_call")
            .map(|(i, _)| i)
            .collect();

        if tool_indices.is_empty() { return; }

        let all_collapsed = tool_indices.iter().all(|i| self.collapsed.contains(i));

        if all_collapsed {
            for i in &tool_indices {
                self.collapsed.remove(i);
            }
        } else {
            for i in &tool_indices {
                self.collapsed.insert(*i);
            }
        }
    }

    fn show_toast(&mut self, msg: String) {
        let color = self.theme.info;
        self.toast = Some((msg, TOAST_DURATION_NORMAL, color));
    }

    fn show_success_toast(&mut self, msg: String) {
        self.toast = Some((msg, TOAST_DURATION_NORMAL, Color::Rgb(0x22, 0xc5, 0x5e)));
    }

    fn show_warning_toast(&mut self, msg: String) {
        self.toast = Some((msg, TOAST_DURATION_NORMAL, Color::Rgb(0xe9, 0xab, 0x2f)));
    }

    fn show_error_toast(&mut self, msg: String) {
        self.toast = Some((msg, TOAST_DURATION_ERROR, Color::Rgb(0xef, 0x44, 0x44)));
    }

    fn thinking_elapsed(&self) -> String {
        if let Some(start) = self.thinking_started {
            let elapsed = (chrono::Utc::now() - start).num_seconds();
            if elapsed < 1 {
                String::new()
            } else if elapsed < 60 {
                format!("{}s", elapsed)
            } else {
                format!("{}m{}s", elapsed / 60, elapsed % 60)
            }
        } else {
            String::new()
        }
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
        constraints.push(Constraint::Length(4)); // input
        constraints.push(Constraint::Length(1)); // footer

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(main_area);

        let mut ci = 0;
        if self.show_splash {
            self.render_splash(f, chunks[ci]);
        } else {
            self.render_messages(f, chunks[ci]);
        }
        // Toast rendered as overlay on top of messages
        if has_toast {
            if let Some((ref msg, _, ref color)) = self.toast.clone() {
                self.render_toast_overlay(f, chunks[ci], msg, *color);
            }
        }
        ci += 1;
        self.render_input(f, chunks[ci]);
        ci += 1;
        self.render_footer(f, chunks[ci]);

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
        if let Some((_, ref mut count, _)) = self.toast {
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
        let t = &self.theme;
            let inner_area = Rect {
                x: area.x,
                y: area.y,
                width: area.width.min(SIDEBAR_WIDTH),
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
                    let symbol = match status.as_str() {
                        "connected" => " ●",
                        "error" => " ●",
                        "needs_auth" => " △",
                        "disabled" => " ○",
                        _ => " •",
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
                        Span::styled(format!(" {}", display_name), muted),
                        Span::styled(format!(" {}", label), muted),
                        Span::raw(" "),
                        Span::styled(symbol, dot),
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
        let (lines, scroll, style) = match &self.diff_viewer {
            Some(v) => v,
            None => return,
        };
        let t = &self.theme;
        let area = f.area();

        // Find hunk and file positions for navigation
        let file_positions: Vec<(usize, &str)> = lines.iter().enumerate()
            .filter(|(_, l)| l.starts_with("--- a/"))
            .filter_map(|(i, l)| {
                l.strip_prefix("--- a/").map(|path| (i, path))
            })
            .collect();

        let hunk_positions: Vec<usize> = lines.iter().enumerate()
            .filter(|(_, l)| l.starts_with("@@"))
            .map(|(i, _)| i)
            .collect();

        let max_lines = (area.height as usize).saturating_sub(6);
        let scroll = *scroll;
        let visible: Vec<&str> = lines.iter().skip(scroll).take(max_lines).map(|s| s.as_str()).collect();
        let total = lines.len();

        let line_num_width = if total >= 10000 { 5 } else if total >= 1000 { 4 } else if total >= 100 { 3 } else if total >= 10 { 2 } else { 1 };
        let content_width = (area.width as usize).saturating_sub(line_num_width + 4);

        // Identify current file
        let current_file = file_positions.iter().rev()
            .find(|(pos, _)| *pos <= scroll)
            .map(|(_, path)| *path)
            .unwrap_or("");

        let style_label = if style == "side_by_side" { "split" } else { "unified" };
        let title = format!(
            " Diff Viewer [{} lines, {} files, {} hunks] [{}] ",
            total, file_positions.len(), hunk_positions.len(), style_label,
        );
        let file_hint = if file_positions.len() > 1 {
            " n/p prev/next file  "
        } else {
            ""
        };
        let status_line = format!(
            " ↑↓/PgUp/PgDn scroll  [ ] hunk jump  v toggle view  s toggle source{} m mark  w wrap:{}  Esc close | {}",
            file_hint, self.diff_wrap_mode, current_file,
        );

        // Split area into sidebar + content if multiple files
        let (sidebar_area, content_area) = if file_positions.len() > 1 {
            let horiz = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(28), Constraint::Min(1)])
                .split(area);
            (horiz[0], horiz[1])
        } else {
            (Rect::default(), area)
        };

        let wrap_mode = &self.diff_wrap_mode;
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .flat_map(|(i, line)| {
                let actual_line = scroll + i + 1;
                let line_num = format!("{:>width$}", actual_line, width = line_num_width);

                let (fg_style, bg_color, num_fg, num_bg) = if line.starts_with('+') && !line.starts_with("+++") {
                    (Style::default().fg(t.diff_add).add_modifier(Modifier::DIM),
                     t.diff_add_bg,
                     t.diff_add,
                     t.diff_add_line_number_bg)
                } else if line.starts_with('-') && !line.starts_with("---") {
                    (Style::default().fg(t.diff_del).add_modifier(Modifier::DIM),
                     t.diff_del_bg,
                     t.diff_del,
                     t.diff_del_line_number_bg)
                } else if line.starts_with("@@") || line.starts_with("--- ") || line.starts_with("+++ ") {
                    (Style::default().fg(t.diff_hunk).add_modifier(Modifier::DIM),
                     t.bg,
                     t.diff_hunk,
                     t.bg)
                } else {
                    (Style::default().fg(t.diff_context),
                     t.diff_context_bg,
                     t.diff_line_number,
                     t.bg)
                };

                let line_content = if wrap_mode == "word" {
                    textwrap::fill(line, content_width)
                } else if wrap_mode == "char" {
                    textwrap::fill(line, content_width)
                } else {
                    line.to_string()
                };

                line_content.lines().enumerate().map(move |(j, wrapped_line)| {
                    let num_display = if j == 0 { line_num.clone() } else { " ".repeat(line_num_width + 1) };
                    ListItem::new(Line::from(vec![
                        Span::styled(num_display, Style::default().fg(num_fg).bg(num_bg)),
                        Span::styled(format!("{}", wrapped_line), fg_style.bg(bg_color)),
                    ]))
                }).collect::<Vec<ListItem>>()
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .title_bottom(status_line)
                .border_style(Style::default().fg(t.primary)),
        );

        f.render_widget(list, content_area);

        // File tree sidebar
        if file_positions.len() > 1 {
            let sidebar_items: Vec<ListItem> = file_positions.iter()
                .map(|(pos, path)| {
                    let is_active = *pos <= scroll && file_positions.iter()
                        .filter(|(p, _)| *p > *pos && *p <= scroll)
                        .next()
                        .is_none();
                    let reviewed = self.reviewed_files.contains(*path);
                    let (fg, bg) = if reviewed {
                        (t.success, t.background_element)
                    } else if is_active {
                        (t.selected_list_item_text, t.background_panel)
                    } else {
                        (t.text, t.background_element)
                    };
                    let marker = if reviewed {
                        " ✓ "
                    } else if is_active {
                        " > "
                    } else {
                        "   "
                    };
                    let file_name = path.rsplit('/').next().unwrap_or(path);
                    ListItem::new(Line::from(vec![
                        Span::styled(marker, Style::default().fg(t.primary)),
                        Span::styled(
                            file_name.to_string(),
                            Style::default().fg(fg).bg(bg).add_modifier(
                                if reviewed { Modifier::DIM } else { Modifier::empty() }
                            ),
                        ),
                    ]))
                })
                .collect();

            let sidebar_widget = List::new(sidebar_items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Files ")
                        .border_style(Style::default().fg(t.border)),
                )
                .style(Style::default().bg(t.background_element));
            f.render_widget(sidebar_widget, sidebar_area);
        }
    }

    fn render_toast_overlay(&self, f: &mut Frame, area: Rect, msg: &str, color: Color) {
        let t = &self.theme;
        let text = Span::styled(
            format!(" {} ", msg),
            Style::default()
                .fg(color)
                .bg(t.background_panel)
                .add_modifier(Modifier::BOLD),
        );
        let w = msg.len() as u16 + 6;
        let overlay = Rect {
            x: area.right().saturating_sub(w.min(area.width)),
            y: area.top().saturating_add(1),
            width: w.min(area.width),
            height: 1,
        };
        f.render_widget(Clear, overlay);
        f.render_widget(
            ratatui::widgets::Paragraph::new(Line::from(text)),
            overlay,
        );
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let t = &self.theme;
        let status_symbol = if self.streaming { "●" } else { "○" };
        let left = Span::styled(
            format!(" {} ", self.model_name),
            Style::default().fg(t.primary).add_modifier(Modifier::BOLD),
        );
        let input_len = self.input.len();
        let char_info = if input_len > 0 {
            format!(" {} chars", input_len)
        } else {
            String::new()
        };
        let elapsed = self.thinking_elapsed();
        let right_text = format!(
            "{}:{}{}{}| {}",
            self.theme_name,
            self.prompt_count,
            char_info,
            if elapsed.is_empty() { String::new() } else { format!(" {}", elapsed) },
            status_symbol,
        );
        let right = Span::styled(
            right_text,
            Style::default().fg(if self.streaming { t.success } else { t.text_muted }),
        );
        let mut spans: Vec<Span> = Vec::new();
        if self.plan_mode {
            spans.push(Span::styled(
                " PLAN ",
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(" │ ", Style::default().fg(t.border)));
        }
        if self.leader_mode {
            spans.push(Span::styled(
                " LEADER ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(" │ ", Style::default().fg(t.border)));
        }
        if !self.agent_name.is_empty() {
            spans.push(Span::styled(
                format!(" {} ", self.agent_name),
                Style::default().fg(t.secondary).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(" │ ", Style::default().fg(t.border)));
        }
        spans.push(left);

        // Pending permission indicator
        if self.pending_perm.is_some() {
            spans.push(Span::styled(" │ ", Style::default().fg(t.border)));
            spans.push(Span::styled(
                " △ perm ",
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            ));
        }

        spans.push(Span::styled("│", Style::default().fg(t.border)));
        spans.push(right);

        // Context tokens bar on the far right
        if self.context_percent > 0 {
            let ctx = Span::styled(
                format!(" {}% ", self.context_percent),
                Style::default().fg(if self.context_percent > 80 { t.warning } else { t.text_muted }),
            );
            spans.push(Span::styled(" │ ", Style::default().fg(t.border)));
            spans.push(ctx);
        }

        let line = Line::from(spans);
        f.render_widget(
            Paragraph::new(line).style(Style::default().bg(t.background_menu)),
            area,
        );
    }

    fn render_splash(&self, f: &mut Frame, area: Rect) {
        let t = &self.theme;
        let splash = r#"
     ███████╗ ██████╗ ██████╗ ██████╗ ███╗   ██╗ ██████╗ ██████╗ ██████╗ ███████╗
     ██╔════╝██╔═══██╗██╔══██╗██╔══██╗████╗  ██║██╔════╝██╔════╝██╔═══██╗██╔════╝
     █████╗  ██║   ██║██████╔╝██████╔╝██╔██╗ ██║██║     ██║     ██║   ██║█████╗
     ██╔══╝  ██║   ██║██╔══██╗██╔══██╗██║╚██╗██║██║     ██║     ██║   ██║██╔══╝
     ██║     ╚██████╔╝██║  ██║██████╔╝██║ ╚████║╚██████╗╚██████╗╚██████╔╝███████╗
     ╚═╝      ╚═════╝ ╚═╝  ╚═╝╚═════╝ ╚═╝  ╚═══╝ ╚═════╝ ╚═════╝ ╚═════╝ ╚══════╝"#;

        let lines: Vec<Line> = splash.lines()
            .map(|l| {
                if l.trim().is_empty() {
                    Line::from(vec![Span::raw("")])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(l, Style::default().fg(t.primary).add_modifier(Modifier::BOLD)),
                    ])
                }
            })
            .collect();

        let splash_height = (lines.len() as u16 + 2).min(area.height);
        let splash_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: splash_height,
        };

        f.render_widget(Clear, splash_area);
        let logo = Paragraph::new(lines)
            .style(Style::default().bg(t.background_panel));
        f.render_widget(logo, splash_area);

        let hint = Span::styled(
            format!("  opencode-rs {} | Press any key to start  ", env!("CARGO_PKG_VERSION")),
            Style::default().fg(t.text_dim).add_modifier(Modifier::DIM),
        );
        let hint_area = Rect {
            x: area.x,
            y: area.y + splash_height,
            width: area.width,
            height: 1,
        };
        if hint_area.y < area.bottom() {
            f.render_widget(Clear, hint_area);
            f.render_widget(Paragraph::new(Line::from(vec![hint])).style(Style::default().bg(t.background_panel)), hint_area);
        }
    }

    fn render_messages(&self, f: &mut Frame, area: Rect) {
        let t = &self.theme;
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
                    if m.role == "tool_result" {
                        let first_line = m.content.lines().next().unwrap_or(&m.content);
                        let remaining = m.content.len() - first_line.len();
                        format!("{} [+{} more - press o to expand]", first_line, remaining)
                    } else {
                        let preview: String = m.content.chars().take(100).collect();
                        format!("{}... [+{} chars collapsed]", preview, m.content.len() - 100)
                    }
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

                let role_indicator = match m.role.as_str() {
                    "tool_call" => Span::styled("⚙", Style::default().fg(t.tool_call).add_modifier(Modifier::DIM)),
                    "tool_result" => Span::styled("↳", Style::default().fg(t.tool_result).add_modifier(Modifier::DIM)),
                    _ => Span::raw(""),
                };

                let mut lines = vec![Line::from(vec![bar.clone(), spinner, role_indicator, pad])];

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
                if m.role == "assistant" || m.role == "reasoning" || m.role == "tool_result" || m.role == "tool_call" {
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

                // Spacer between messages
                lines.push(Line::from(vec![bar, Span::raw("")]));

                // Age-based fade-in: new messages start dimmed, fade over 10 frames
                let fade_mod = if m.age < 10 {
                    let dim_level = (10 - m.age) as f32 / 10.0;
                    if dim_level > 0.5 {
                        Modifier::DIM
                    } else {
                        Modifier::empty()
                    }
                } else {
                    Modifier::empty()
                };
                // Apply thinking_opacity dimming to reasoning messages
                let thinking_mod = if m.role == "reasoning" && t.thinking_opacity < 1.0 {
                    Modifier::DIM
                } else {
                    Modifier::empty()
                };
                ListItem::new(lines).style(Style::default().bg(bg_color).add_modifier(fade_mod).add_modifier(thinking_mod))
            })
            .collect();

        let messages = List::new(items)
            .style(Style::default().bg(t.background_panel));

        // Split area: list takes all but rightmost 2 columns for scrollbar
        let scrollbar_width = 2u16;
        let list_width = area.width.saturating_sub(scrollbar_width);
        let list_area = Rect { width: list_width, ..area };
        let scrollbar_area = Rect {
            x: area.right().saturating_sub(scrollbar_width),
            width: scrollbar_width,
            ..area
        };

        f.render_widget(messages, list_area);

        // Custom scrollbar indicator
        if total > max_visible {
            let scrollbar_track = Span::styled(
                "░".repeat(scrollbar_area.height as usize),
                Style::default().fg(t.border).add_modifier(Modifier::DIM),
            );
            let scrollbar_bg = Paragraph::new(scrollbar_track)
                .style(Style::default().bg(t.background_panel));
            f.render_widget(scrollbar_bg, scrollbar_area);

            let scroll_ratio = start as f64 / total.saturating_sub(max_visible) as f64;
            let thumb_pos = (scroll_ratio * scrollbar_area.height.saturating_sub(1) as f64) as u16;
            let thumb = Span::styled(
                "██",
                Style::default().fg(t.text_muted),
            );
            let thumb_area = Rect {
                y: scrollbar_area.y + thumb_pos.min(scrollbar_area.height.saturating_sub(1)),
                height: 1,
                ..scrollbar_area
            };
            f.render_widget(Paragraph::new(thumb), thumb_area);
        }

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
        let text_style = Style::default().fg(theme.markdown_text);
        let heading_style = Style::default().fg(theme.markdown_heading).add_modifier(Modifier::BOLD);
        let block_quote_style = Style::default().fg(theme.markdown_block_quote).add_modifier(Modifier::DIM);
        let code_style = Style::default().fg(theme.markdown_code);
        let link_style = Style::default().fg(theme.markdown_link);
        let link_text_style = Style::default().fg(theme.markdown_link_text);
        let emph_style = Style::default().fg(theme.markdown_emph);
        let strong_style = Style::default().fg(theme.markdown_strong);
        let hr_style = Style::default().fg(theme.markdown_horizontal_rule).add_modifier(Modifier::DIM);
        let list_style = Style::default().fg(theme.markdown_list_item);
        let enum_style = Style::default().fg(theme.markdown_list_enumeration);
        let _code_block_style = Style::default().fg(theme.markdown_code_block);
        let diff_add = Style::default().fg(theme.diff_add).bg(theme.diff_add_bg).add_modifier(Modifier::DIM);
        let diff_del = Style::default().fg(theme.diff_del).bg(theme.diff_del_bg).add_modifier(Modifier::DIM);
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
            } else if line.starts_with("### ") {
                let text = &line[4..];
                let wrapped = textwrap::fill(text, (width as usize).saturating_sub(4));
                for wl in wrapped.lines() {
                    out.push(Line::from(vec![Span::styled(format!("  ### {}", wl), heading_style)]));
                }
            } else if line.starts_with("## ") {
                let text = &line[3..];
                let wrapped = textwrap::fill(text, (width as usize).saturating_sub(4));
                for wl in wrapped.lines() {
                    out.push(Line::from(vec![Span::styled(format!("  ## {}", wl), heading_style)]));
                }
            } else if line.starts_with("# ") {
                let text = &line[2..];
                let wrapped = textwrap::fill(text, (width as usize).saturating_sub(4));
                for wl in wrapped.lines() {
                    out.push(Line::from(vec![Span::styled(format!("  # {}", wl), heading_style)]));
                }
            } else if line.starts_with("> ") {
                let text = &line[2..];
                let wrapped = textwrap::fill(text, (width as usize).saturating_sub(4));
                for wl in wrapped.lines() {
                    out.push(Line::from(vec![Span::styled(format!("  ▎{}", wl), block_quote_style)]));
                }
            } else if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
                let marker = &line[..1];
                let text = &line[2..];
                let wrapped = textwrap::fill(text, (width as usize).saturating_sub(4));
                for (j, wl) in wrapped.lines().enumerate() {
                    if j == 0 {
                        out.push(Line::from(vec![
                            Span::styled(format!("  {}", marker), list_style),
                            Span::styled(format!(" {}", wl), text_style),
                        ]));
                    } else {
                        out.push(Line::from(vec![
                            Span::raw("    "),
                            Span::styled(wl.to_string(), text_style),
                        ]));
                    }
                }
            } else if line.starts_with(|c: char| c.is_ascii_digit()) && line.contains(". ") {
                if let Some(dot_pos) = line.find(". ") {
                    let num = &line[..=dot_pos];
                    let text = &line[dot_pos+2..];
                    let wrapped = textwrap::fill(text, (width as usize).saturating_sub(4));
                    for (j, wl) in wrapped.lines().enumerate() {
                        if j == 0 {
                            out.push(Line::from(vec![
                                Span::styled(format!("  {}", num), enum_style),
                                Span::styled(format!("{}", wl), text_style),
                            ]));
                        } else {
                            out.push(Line::from(vec![
                                Span::raw("    "),
                                Span::styled(wl.to_string(), text_style),
                            ]));
                        }
                    }
                } else {
                    Self::render_markdown_line(line, width, out, text_style, code_style, link_style, link_text_style, emph_style, strong_style);
                }
            } else if line.trim() == "---" || line.trim() == "***" || line.trim() == "___" {
                out.push(Line::from(vec![Span::styled(format!("  {}", line.trim()), hr_style)]));
            } else {
                Self::render_markdown_line(line, width, out, text_style, code_style, link_style, link_text_style, emph_style, strong_style);
            }
        }

        if in_code && !code_buf.is_empty() {
            Self::render_code_block(&code_buf, width, &code_lang, out, theme);
        }
    }

    fn render_markdown_line(line: &str, width: usize, out: &mut Vec<Line>,
        _text_style: Style, code_style: Style, link_style: Style, link_text_style: Style,
        emph_style: Style, strong_style: Style) {
        let wrapped = textwrap::fill(line, width as usize);
        for wl in wrapped.lines() {
            let mut spans = vec![Span::raw("  ")];
            let mut i = 0;
            let chars: Vec<char> = wl.chars().collect();
            while i < chars.len() {
                // Inline link [text](url)
                if chars[i] == '[' {
                    let text_start = i + 1;
                    let mut j = i + 1;
                    while j < chars.len() && chars[j] != ']' { j += 1; }
                    if j < chars.len() && j + 1 < chars.len() && chars[j + 1] == '(' {
                        let url_start = j + 2;
                        let mut k = url_start;
                        while k < chars.len() && chars[k] != ')' { k += 1; }
                        if k < chars.len() {
                            let text: String = chars[text_start..j].iter().collect();
                            let url: String = chars[url_start..k].iter().collect();
                            spans.push(Span::styled(text, link_text_style));
                            spans.push(Span::styled(format!("({})", url), link_style));
                            i = k + 1;
                            continue;
                        }
                    }
                    spans.push(Span::raw(chars[i].to_string()));
                    i += 1;
                    continue;
                }
                // Inline code
                if chars[i] == '`' {
                    let start = i;
                    i += 1;
                    while i < chars.len() && chars[i] != '`' { i += 1; }
                    if i < chars.len() { i += 1; }
                    let s: String = chars[start..i].iter().collect();
                    spans.push(Span::styled(s, code_style));
                    continue;
                }
                // Bold **text**
                if i + 1 < chars.len() && chars[i] == '*' && chars[i+1] == '*' {
                    let start = i;
                    i += 2;
                    while i + 1 < chars.len() && !(chars[i] == '*' && chars[i+1] == '*') { i += 1; }
                    if i + 1 < chars.len() { i += 2; } else { i += 1; }
                    let s: String = chars[start..i].iter().collect();
                    spans.push(Span::styled(s, strong_style));
                    continue;
                }
                // Italic *text*
                if chars[i] == '*' {
                    let start = i;
                    i += 1;
                    while i < chars.len() && chars[i] != '*' { i += 1; }
                    if i < chars.len() { i += 1; }
                    let s: String = chars[start..i].iter().collect();
                    spans.push(Span::styled(s, emph_style));
                    continue;
                }
                spans.push(Span::raw(chars[i].to_string()));
                i += 1;
            }
            out.push(Line::from(spans));
        }
    }

    fn render_code_block(code: &str, width: usize, lang: &str, out: &mut Vec<Line>, theme: &Theme) {
        let _context_style = Style::default().fg(theme.diff_context).bg(theme.diff_context_bg);
        let _code_style = Style::default().fg(theme.dim).add_modifier(Modifier::DIM);
        let diff_add = Style::default().fg(theme.diff_add).bg(theme.diff_add_bg).add_modifier(Modifier::DIM);
        let diff_del = Style::default().fg(theme.diff_del).bg(theme.diff_del_bg).add_modifier(Modifier::DIM);
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
                    let line_with_bg: Vec<Span> = line_spans.into_iter().map(|s| {
                        Span::styled(s.to_string(), Style::default().fg(s.style.fg.unwrap_or(theme.diff_context)).bg(theme.diff_context_bg))
                    }).collect();
                    out.push(Line::from(line_with_bg));
                }
            }
        }
    }

    fn syntax_highlight_line(line: &str, lang: &str, theme: &Theme) -> Vec<Span<'static>> {
        if line.is_empty() {
            return Vec::new();
        }

        let _fn_style = Style::default().fg(theme.syntax_function);
        let kw_style = Style::default().fg(theme.syntax_keyword);
        let str_style = Style::default().fg(theme.syntax_string);
        let comment_style = Style::default().fg(theme.syntax_comment);
        let num_style = Style::default().fg(theme.syntax_number);
        let builtin_style = Style::default().fg(theme.syntax_builtin);
        let var_style = Style::default().fg(theme.syntax_variable);
        let ty_style = Style::default().fg(theme.syntax_type);
        let op_style = Style::default().fg(theme.syntax_operator);
        let punct_style = Style::default().fg(theme.syntax_punctuation);

        let line = line.trim_end();
        let (comment_prefix, is_comment_line) = Self::get_comment_info(line, lang);
        if is_comment_line {
            return vec![Span::styled(line.to_string(), comment_style)];
        }

        let keywords = Self::get_keywords(lang);
        let types = Self::get_types(lang);
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
                } else if types.contains(&word.as_str()) {
                    spans.push(Span::styled(word, ty_style));
                } else if Self::is_builtin(&word, lang) {
                    spans.push(Span::styled(word, builtin_style));
                } else if word.chars().next().map_or(false, |c| c.is_uppercase()) {
                    spans.push(Span::styled(word, var_style));
                } else {
                    spans.push(Span::raw(word));
                }
                continue;
            }

            // Punctuation / operators
            let ch = chars[i];
            if "{}()[]".contains(ch) {
                spans.push(Span::styled(ch.to_string(), punct_style));
            } else if "+-*/%=!<>&|^~".contains(ch) {
                spans.push(Span::styled(ch.to_string(), op_style));
            } else {
                spans.push(Span::raw(ch.to_string()));
            }
            i += 1;
        }

        spans
    }
    fn get_comment_info(line: &str, lang: &str) -> (Option<&'static str>, bool) {
        let lang = crate::util::filetype::normalize_language(lang);
        let (single, can_be_inline): (&str, bool) = match lang {
            "rust" | "go" | "c" | "cpp" | "java" | "javascript" | "typescript" | "swift" | "kotlin" | "scala" | "dart" | "zig" => ("//", true),
            "python" | "r" | "ruby" | "yaml" | "toml" | "ini" | "cfg" | "perl" | "elixir" | "crystal" | "nim" => ("#", true),
            "lua" | "sql" | "haskell" | "elm" | "fsharp" | "erlang" => ("--", true),
            "clojure" | "lisp" | "scheme" => (";", true),
            "html" | "xml" | "svg" => ("<!--", false),
            "php" => ("//", true),
            "bash" | "shell" | "sh" | "zsh" | "fish" | "makefile" | "dockerfile" | "cmake" | "gradle" | "terraform" | "hcl" => ("#", true),
            "powershell" | "ps1" => ("#", true),
            "batch" | "bat" | "cmd" => ("REM", false),
            "protobuf" | "proto" => ("//", true),
            "graphql" | "gql" => ("#", true),
            "latex" | "tex" => ("%", true),
            "diff" | "patch" => ("//", true),
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
        "bash" => &["if", "then", "else", "elif", "fi", "case", "esac", "for", "while", "until", "do", "done", "in", "select", "function", "return", "exit", "break", "continue", "declare", "local", "export", "readonly", "unset", "eval", "exec", "shift", "source", "trap", "type", "typeset", "ulimit", "umask", "wait", "getopts", "let", "test"],
        "sql" => &["SELECT", "FROM", "WHERE", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE", "TABLE", "ALTER", "DROP", "INDEX", "VIEW", "JOIN", "INNER", "LEFT", "RIGHT", "OUTER", "FULL", "CROSS", "ON", "AND", "OR", "NOT", "IN", "BETWEEN", "LIKE", "IS", "NULL", "ORDER", "BY", "GROUP", "HAVING", "LIMIT", "OFFSET", "DISTINCT", "AS", "UNION", "ALL", "EXISTS", "CASE", "WHEN", "THEN", "ELSE", "END", "CAST", "COUNT", "SUM", "AVG", "MIN", "MAX", "COALESCE", "NULLIF", "BEGIN", "COMMIT", "ROLLBACK", "SAVEPOINT", "GRANT", "REVOKE", "TRIGGER", "PROCEDURE", "FUNCTION", "PACKAGE", "CURSOR", "LOOP", "FETCH", "CLOSE", "IF", "ELSIF", "WHILE", "FOR", "RETURN", "DECLARE", "EXCEPTION", "RAISE"],
        "perl" => &["if", "unless", "elsif", "else", "given", "when", "default", "for", "foreach", "while", "until", "do", "continue", "last", "next", "redo", "goto", "return", "my", "our", "state", "local", "use", "require", "no", "package", "sub", "BEGIN", "CHECK", "INIT", "END", "eval", "die", "warn", "caller", "wantarray", "bless", "ref", "tie", "untie", "tied", "defined", "undef", "delete", "exists", "lock"],
        "csharp" => &["abstract", "as", "base", "bool", "break", "byte", "case", "catch", "char", "checked", "class", "const", "continue", "decimal", "default", "delegate", "do", "double", "else", "enum", "event", "explicit", "extern", "false", "finally", "fixed", "float", "for", "foreach", "goto", "if", "implicit", "in", "int", "interface", "internal", "is", "lock", "long", "namespace", "new", "null", "object", "operator", "out", "override", "params", "private", "protected", "public", "readonly", "ref", "return", "sbyte", "sealed", "short", "sizeof", "stackalloc", "static", "string", "struct", "switch", "this", "throw", "true", "try", "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using", "var", "virtual", "void", "volatile", "while"],
        "elixir" => &["true", "false", "nil", "def", "defp", "defmodule", "defstruct", "defprotocol", "defimpl", "defexception", "defmacro", "defmacrop", "defguard", "defguardp", "if", "else", "unless", "cond", "case", "receive", "send", "spawn", "raise", "throw", "try", "rescue", "catch", "after", "else", "import", "alias", "use", "require", "quote", "unquote", "super", "fn", "do", "end", "for", "with", "when", "in", "not", "and", "or", "xor", "is_atom", "is_binary", "is_bitstring", "is_boolean", "is_exception", "is_float", "is_function", "is_integer", "is_list", "is_map", "is_number", "is_pid", "is_port", "is_reference", "is_struct", "is_tuple"],
        "zig" => &["const", "var", "fn", "pub", "export", "extern", "inline", "noinline", "comptime", "volatile", "align", "linksection", "threadlocal", "allowzero", "callconv", "addrspace", "usingnamespace", "test", "return", "if", "else", "switch", "for", "while", "continue", "break", "defer", "errdefer", "try", "catch", "anyerror", "anytype", "anyframe", "null", "undefined", "true", "false", "struct", "enum", "union", "error", "opaque", "type", "packed", "and", "or", "orelse", "_"],
        "html" => &["html", "head", "body", "div", "span", "p", "a", "img", "ul", "ol", "li", "table", "tr", "td", "th", "form", "input", "button", "select", "option", "textarea", "label", "h1", "h2", "h3", "h4", "h5", "h6", "header", "footer", "nav", "section", "article", "aside", "main", "meta", "link", "script", "style", "title", "br", "hr", "pre", "code", "blockquote", "em", "strong", "i", "b", "u", "s", "small", "sub", "sup", "iframe", "video", "audio", "canvas", "svg", "path", "circle", "rect", "line", "text"],
        "css" => &["color", "background", "background-color", "background-image", "margin", "padding", "border", "font", "font-size", "font-weight", "font-family", "text-align", "text-decoration", "display", "position", "top", "left", "right", "bottom", "width", "height", "max-width", "min-width", "max-height", "min-height", "overflow", "flex", "grid", "align-items", "justify-content", "gap", "z-index", "opacity", "transform", "transition", "animation", "box-shadow", "border-radius", "cursor", "list-style", "outline", "visibility", "content", "@import", "@media", "@keyframes", "@font-face"],
        "clojure" => &["def", "defn", "defmacro", "defmethod", "defmulti", "defprotocol", "defrecord", "deftype", "defonce", "fn", "let", "letfn", "if", "if-not", "when", "when-not", "when-let", "when-first", "cond", "case", "do", "loop", "recur", "for", "doseq", "dotimes", "while", "with-open", "with-local-vars", "binding", "ns", "in-ns", "require", "use", "import", "refer", "declare", "->", "->>", "as->", "some->", "map", "filter", "reduce", "comp", "partial", "apply", "juxt", "iterate", "repeat", "repeatedly", "nil?", "some?", "every?", "not", "and", "or", "try", "catch", "finally", "throw", "ex-info", "ex-data"],
        "erlang" => &["module", "export", "import", "define", "record", "type", "spec", "callback", "if", "case", "of", "when", "receive", "after", "try", "catch", "throw", "error", "exit", "fun", "spawn", "send", "self", "register", "whereis", "registered", "exit", "link", "unlink", "monitor", "demonitor", "list_to_atom", "atom_to_list", "binary_to_list", "list_to_binary", "tuple_to_list", "list_to_tuple", "integer_to_list", "list_to_integer", "float_to_list", "list_to_float", "abs", "round", "trunc", "length", "size", "hd", "tl", "element", "setelement", "tuple_size", "byte_size", "bit_size", "is_atom", "is_binary", "is_bitstring", "is_boolean", "is_float", "is_function", "is_integer", "is_list", "is_map", "is_number", "is_pid", "is_port", "is_record", "is_reference", "is_tuple", "true", "false", "ok", "error", "undefined"],
        "diff" => &["---", "+++", "@@", "diff", "index", "new", "deleted", "modified", "rename", "copy"],
        "fsharp" => &["let", "let!", "use", "use!", "do", "do!", "yield", "yield!", "return", "return!", "match", "with", "when", "if", "then", "else", "elif", "while", "for", "in", "to", "downto", "step", "module", "namespace", "open", "type", "val", "member", "static", "override", "abstract", "default", "new", "inherit", "base", "class", "struct", "interface", "enum", "union", "record", "of", "function", "fun", "inline", "rec", "and", "or", "not", "true", "false", "null", "lazy", "async", "task", "seq", "async", "try", "with", "finally", "raise", "failwith", "failwithf", "nullArg", "invalidArg", "invalidOp"],
        "nim" => &["type", "var", "let", "const", "proc", "func", "method", "iterator", "template", "macro", "converter", "block", "if", "elif", "else", "case", "of", "when", "try", "except", "finally", "raise", "for", "while", "break", "continue", "return", "yield", "discard", "include", "import", "export", "from", "as", "mixin", "bind", "static", "deferred", "using", "cast", "addr", "unsafeAddr", "varargs", "distinct", "ref", "ptr", "object", "enum", "tuple", "array", "seq", "set", "range", "openArray", "string", "cstring", "true", "false", "nil", "result"],
        "crystal" => &["alias", "as", "as?", "asm", "begin", "break", "case", "class", "def", "do", "else", "elsif", "end", "ensure", "enum", "extend", "false", "for", "fun", "if", "ifdef", "in", "include", "instance_sizeof", "is_a?", "lib", "macro", "module", "next", "nil", "nil?", "of", "out", "pointerof", "private", "protected", "public", "raise", "readonly", "record", "redo", "require", "rescue", "return", "self", "sizeof", "struct", "super", "then", "true", "type", "typeof", "union", "uninitialized", "unless", "until", "verbatim", "when", "while", "with", "yield", "__DIR__", "__FILE__", "__LINE__"],
        "powershell" => &["function", "filter", "param", "begin", "process", "end", "if", "else", "elseif", "switch", "for", "foreach", "while", "do", "until", "continue", "break", "return", "throw", "try", "catch", "finally", "trap", "in", "not", "and", "or", "xor", "is", "as", "match", "notmatch", "contains", "notcontains", "like", "notlike", "ceq", "cne", "cgt", "clt", "cge", "cle", "eq", "ne", "gt", "lt", "ge", "le", "replace", "split", "join", "array", "hashtable", "scriptblock", "type", "using", "namespace", "class", "enum", "data", "dynamicparam", "exit", "filter", "workflow", "parallel", "sequence", "inlinescript", "configuration", "Import-Module", "Write-Host", "Write-Output", "Write-Error", "Write-Verbose", "Write-Debug", "Write-Warning"],
        _ => &[],
        }
    }

    fn get_types(lang: &str) -> &'static [&'static str] {
        let lang = crate::util::filetype::normalize_language(lang);
        match lang {
            "rust" => &["i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "f32", "f64", "bool", "char", "str", "String", "Vec", "HashMap", "HashSet", "Option", "Result", "Box", "Rc", "Arc", "Cell", "RefCell", "Mutex", "Path", "PathBuf", "OsString", "Duration", "io::Error", "io::Result"],
            "go" => &["bool", "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64", "float32", "float64", "complex64", "complex128", "string", "byte", "rune", "error", "any"],
            "python" => &["int", "float", "str", "bool", "bytes", "list", "dict", "tuple", "set", "frozenset", "None", "Any", "Optional", "List", "Dict", "Tuple", "Set", "Callable", "TypeVar", "Generic", "Protocol"],
            "javascript" | "typescript" => &["number", "string", "boolean", "undefined", "null", "symbol", "bigint", "object", "any", "never", "void", "unknown", "Array", "Promise", "Record", "Partial", "Required", "Pick", "Omit", "Readonly", "Exclude", "Extract", "NonNullable"],
            "java" => &["byte", "short", "int", "long", "float", "double", "boolean", "char", "String", "Object", "List", "ArrayList", "Map", "HashMap", "Set", "HashSet", "Optional", "Integer", "Long", "Double", "Boolean", "Character", "Void", "Throwable", "Exception", "RuntimeException"],
            "c" | "cpp" => &["int", "long", "short", "char", "float", "double", "bool", "void", "size_t", "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "string", "vector", "map", "set", "pair", "optional", "unique_ptr", "shared_ptr"],
            "ruby" => &["String", "Integer", "Float", "Symbol", "Array", "Hash", "Range", "Regexp", "Time", "Date", "DateTime", "Proc", "Lambda", "Method", "Class", "Module", "Object", "BasicObject", "Kernel", "NilClass", "TrueClass", "FalseClass", "Enumerator"],
            "php" => &["int", "float", "string", "bool", "array", "object", "void", "null", "mixed", "never", "self", "static", "parent", "callable", "iterable", "false", "true", "Stringable", "ArrayAccess", "Traversable", "Countable"],
            "swift" => &["Int", "Int8", "Int16", "Int32", "Int64", "UInt", "UInt8", "UInt16", "UInt32", "UInt64", "Float", "Float32", "Float64", "Double", "Bool", "String", "Character", "Optional", "Array", "Dictionary", "Set", "Data", "Date", "URL", "Error", "Result", "Any", "AnyObject", "Codable", "Equatable", "Hashable", "Comparable"],
            "kotlin" => &["Int", "Long", "Short", "Byte", "Double", "Float", "Boolean", "Char", "String", "Any", "Unit", "Nothing", "Array", "List", "MutableList", "Set", "MutableSet", "Map", "MutableMap", "Pair", "Triple", "Iterable", "Collection", "Comparable", "CharSequence", "Number"],
            "scala" => &["Int", "Long", "Short", "Byte", "Double", "Float", "Boolean", "Char", "String", "Unit", "Nothing", "Any", "AnyVal", "AnyRef", "Option", "Some", "None", "Either", "Left", "Right", "Try", "Success", "Failure", "List", "Set", "Map", "Seq", "Array", "Future", "ExecutionContext"],
            "lua" => &["nil", "boolean", "number", "string", "table", "function", "thread", "userdata"],
            "haskell" => &["Bool", "True", "False", "Maybe", "Just", "Nothing", "Either", "Left", "Right", "IO", "Int", "Integer", "Float", "Double", "Char", "String", "Ord", "Eq", "Show", "Read", "Enum", "Bounded", "Num", "Integral", "Floating", "Functor", "Applicative", "Monad", "Foldable", "Traversable"],
            "dart" => &["int", "double", "num", "bool", "String", "Object", "dynamic", "void", "Never", "Null", "List", "Set", "Map", "Record", "Comparable", "Exception", "Error", "Future", "Stream", "Iterable", "Function", "Symbol", "Type"],
            "bash" => &["true", "false"],
            "sql" => &["INTEGER", "INT", "BIGINT", "SMALLINT", "TINYINT", "REAL", "FLOAT", "DOUBLE", "DECIMAL", "NUMERIC", "BOOLEAN", "CHAR", "VARCHAR", "TEXT", "CLOB", "BLOB", "DATE", "TIMESTAMP", "DATETIME", "TIME", "INTERVAL", "JSON", "UUID", "ARRAY", "MONEY", "SERIAL"],
            "perl" => &["Scalar", "Array", "Hash", "Ref", "Code", "Glob", "IO"],
            "csharp" => &["bool", "byte", "sbyte", "short", "ushort", "int", "uint", "long", "ulong", "nint", "nuint", "float", "double", "decimal", "char", "string", "object", "dynamic", "void", "var", "DateTime", "TimeSpan", "Guid", "Task", "Task<T>", "ValueTask", "IEnumerable", "IEnumerator", "ICollection", "IList", "IDictionary", "List<T>", "Dictionary<TKey,TValue>", "HashSet<T>", "Queue<T>", "Stack<T>", "LinkedList<T>", "ObservableCollection<T>", "StringBuilder", "Stream", "MemoryStream", "FileStream", "Exception", "InvalidOperationException", "ArgumentNullException", "ArgumentException", "HttpClient", "CancellationToken"],
            "elixir" => &["Integer", "Float", "Atom", "String", "List", "Map", "Tuple", "Regex", "PID", "Port", "Reference", "Function", "Module", "GenServer", "Agent", "Task", "Supervisor", "Registry", "ETS"],
            "zig" => &["u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f16", "f32", "f64", "f128", "bool", "void", "noreturn", "type", "anyerror", "anytype", "anyframe", "comptime_int", "comptime_float", "isize", "usize", "c_int", "c_uint", "c_long", "c_ulong", "c_longlong", "c_ulonglong", "c_short", "c_ushort"],
            "html" => &["string", "number", "boolean", "object", "Array", "Function"],
            "css" => &["string", "number", "color", "url"],
            "clojure" => &["Atom", "Ref", "Agent", "Var", "Keyword", "Symbol", "Ratio", "BigInt", "BigDecimal", "IPersistentMap", "IPersistentVector", "IPersistentList", "IPersistentSet", "ISeq", "LazySeq", "MultiFn", "Protocol", "Type", "Record", "Delay", "Future", "Promise", "Queue"],
            "erlang" => &["pid", "port", "ref", "atom", "binary", "bitstring", "boolean", "byte", "char", "float", "integer", "list", "map", "number", "string", "tuple", "module", "mfa", "function", "non_neg_integer", "pos_integer", "neg_integer", "timeout", "iodata", "iolist", "term", "any"],
            "diff" => &["file", "hunk", "line"],
            "fsharp" => &["int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64", "float", "float32", "float64", "double", "decimal", "bool", "char", "byte", "sbyte", "string", "unit", "obj", "exn", "list", "array", "seq", "option", "value option", "Result", "Choice", "Map", "Set", "async", "task", "Async<T>", "Task<T>", "IEnumerable", "IDisposable", "IComparable", "IEquatable"],
            "nim" => &["int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64", "float", "float32", "float64", "bool", "char", "string", "cstring", "pointer", "typedesc", "void", "auto", "any", "untyped", "typed", "lent", "sink", "seq", "array", "openArray", "set", "range", "tuple", "object", "enum", "ref", "ptr", "var", "distinct", "SomeSignedInt", "SomeUnsignedInt", "SomeInteger", "SomeFloat", "SomeNumber", "Ordinal", "Natural", "Positive"],
            "crystal" => &["Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64", "Float32", "Float64", "Bool", "Char", "String", "Symbol", "Nil", "Pointer", "Slice", "Bytes", "Array", "Set", "Tuple", "NamedTuple", "Hash", "Regex", "Range", "Enumerator", "Iterator", "Proc", "Deque", "Struct", "Union", "Number", "Value", "Reference", "Class", "Module", "Generic"],
            "powershell" => &["string", "int", "long", "double", "decimal", "bool", "char", "byte", "datetime", "array", "hashtable", "psobject", "pscustomobject", "xml", "regex", "scriptblock", "switchparameter", "void", "type", "int32", "int64", "single", "uint32", "uint64", "timespan", "guid", "uri", "version"],
            _ => &[],
        }
    }

    fn is_builtin(word: &str, lang: &str) -> bool {
        let lang = crate::util::filetype::normalize_language(lang);
        match lang {
        "python" => matches!(word, "print" | "len" | "range" | "type" | "str" | "int" | "float" | "list" | "dict" | "set" | "tuple" | "bool" | "super" | "self" | "open" | "map" | "filter" | "zip" | "enumerate" | "sorted" | "reversed" | "any" | "all" | "sum" | "min" | "max" | "abs" | "round" | "isinstance" | "hasattr" | "getattr" | "setattr" | "ValueError" | "TypeError" | "KeyError" | "Exception" | "BaseException" | "object" | "property" | "staticmethod" | "classmethod" | "input" | "eval" | "exec" | "compile" | "repr" | "format" | "id" | "hash" | "help" | "dir" | "vars" | "locals" | "globals" | "iter" | "next" | "slice" | "bin" | "oct" | "hex" | "ord" | "chr" | "divmod" | "pow"),
        "javascript" | "typescript" => matches!(word, "console" | "log" | "error" | "warn" | "require" | "module" | "exports" | "process" | "Buffer" | "setTimeout" | "setInterval" | "fetch" | "Promise" | "Array" | "Object" | "String" | "Number" | "Boolean" | "Map" | "Set" | "Symbol" | "JSON" | "Math" | "Date" | "RegExp" | "Error" | "undefined" | "null" | "true" | "false" | "window" | "document" | "globalThis" | "exports" | "describe" | "it" | "test" | "expect" | "jest" | "console" | "parseInt" | "parseFloat" | "isNaN" | "isFinite" | "decodeURI" | "encodeURI" | "localStorage" | "sessionStorage"),
        "ruby" => matches!(word, "puts" | "print" | "p" | "require" | "include" | "extend" | "attr_accessor" | "attr_reader" | "attr_writer" | "private" | "protected" | "public" | "raise" | "fail" | "catch" | "throw" | "lambda" | "proc" | "eval" | "loop" | "sleep" | "gets" | "chomp" | "inspect" | "to_s" | "to_i" | "to_f" | "nil?" | "empty?" | "length" | "size" | "each" | "map" | "select" | "reject" | "reduce" | "inject" | "sort" | "uniq" | "first" | "last"),
        "php" => matches!(word, "echo" | "print" | "die" | "exit" | "isset" | "unset" | "empty" | "require" | "require_once" | "include" | "include_once" | "defined" | "array" | "count" | "strlen" | "strpos" | "substr" | "explode" | "implode" | "json_encode" | "json_decode" | "preg_match" | "sprintf" | "var_dump" | "error_log" | "header" | "session_start" | "setcookie" | "is_null" | "is_numeric" | "PHP_EOL" | "true" | "false" | "null"),
        "rust" => matches!(word, "std" | "core" | "alloc" | "println" | "print" | "eprintln" | "eprint" | "format" | "write" | "writeln" | "vec" | "format_args" | "assert" | "assert_eq" | "assert_ne" | "panic" | "unreachable" | "unimplemented" | "todo" | "dbg" | "include_str" | "include_bytes" | "file" | "line" | "column" | "cfg" | "env" | "option_env" | "concat" | "stringify"),
        "go" => matches!(word, "fmt" | "Print" | "Printf" | "Println" | "Sprint" | "Sprintf" | "Sprintln" | "Fprint" | "Fprintf" | "Fprintln" | "Errorf" | "append" | "copy" | "delete" | "len" | "cap" | "make" | "new" | "close" | "panic" | "recover" | "print" | "println" | "error" | "string"),
        "java" => matches!(word, "System" | "out" | "err" | "in" | "println" | "print" | "printf" | "String" | "Integer" | "Double" | "Math" | "Arrays" | "Collections" | "Objects" | "Optional" | "Stream" | "Collectors" | "List" | "ArrayList" | "Map" | "HashMap" | "Set" | "HashSet" | "Thread" | "Runnable" | "Comparator" | "Comparable" | "Exception" | "RuntimeException" | "IllegalArgumentException" | "NullPointerException"),
        "swift" => matches!(word, "print" | "debugPrint" | "dump" | "fatalError" | "precondition" | "preconditionFailure" | "assert" | "assertionFailure" | "abs" | "min" | "max" | "zip" | "stride" | "sequence" | "repeatElement" | "type" | "sizeof" | "UIColor" | "NSObject" | "CGRect" | "CGSize" | "CGPoint"),
        "kotlin" => matches!(word, "println" | "print" | "readLine" | "require" | "check" | "error" | "TODO" | "run" | "let" | "apply" | "also" | "with" | "use" | "repeat" | "listOf" | "setOf" | "mapOf" | "arrayOf" | "emptyList" | "emptyMap" | "emptySet" | "mutableListOf" | "mutableMapOf" | "mutableSetOf" | "sequenceOf"),
        "bash" => matches!(word, "echo" | "printf" | "read" | "source" | "export" | "local" | "declare" | "typeset" | "test" | "let" | "eval" | "exec" | "exit" | "return" | "break" | "continue" | "shift" | "cd" | "pwd" | "ls" | "mkdir" | "rm" | "cp" | "mv" | "cat" | "grep" | "sed" | "awk" | "find" | "xargs" | "sort" | "uniq" | "wc" | "head" | "tail" | "cut" | "tr" | "diff" | "chmod" | "chown"),
        "sql" => matches!(word, "SELECT" | "FROM" | "WHERE" | "INSERT" | "INTO" | "VALUES" | "UPDATE" | "SET" | "DELETE" | "CREATE" | "TABLE" | "ALTER" | "DROP" | "INDEX" | "VIEW" | "JOIN" | "INNER" | "LEFT" | "RIGHT" | "OUTER" | "FULL" | "CROSS" | "ON" | "AND" | "OR" | "NOT" | "IN" | "BETWEEN" | "LIKE" | "IS" | "NULL" | "TRUE" | "FALSE" | "ORDER" | "BY" | "GROUP" | "HAVING" | "LIMIT" | "OFFSET" | "DISTINCT" | "AS" | "UNION" | "ALL" | "EXISTS" | "CASE" | "WHEN" | "THEN" | "ELSE" | "END" | "CAST" | "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" | "COALESCE" | "NULLIF"),
        "perl" => matches!(word, "print" | "say" | "warn" | "die" | "exit" | "shift" | "pop" | "push" | "unshift" | "splice" | "keys" | "values" | "exists" | "defined" | "delete" | "length" | "substr" | "index" | "rindex" | "join" | "split" | "map" | "grep" | "sort" | "reverse" | "chomp" | "chr" | "ord" | "uc" | "lc" | "open" | "close" | "read" | "write" | "tell" | "seek" | "require" | "use" | "no" | "local" | "my" | "our" | "state" | "bless" | "ref" | "tie" | "untie"),
        "csharp" => matches!(word, "Console" | "WriteLine" | "Write" | "ReadLine" | "Read" | "Convert" | "Math" | "String" | "StringBuilder" | "Int32" | "Int64" | "Double" | "Boolean" | "DateTime" | "Guid" | "Task" | "Task<T>" | "async" | "await" | "yield" | "return" | "throw" | "nameof" | "sizeof" | "typeof" | "default" | "new" | "this" | "base" | "null" | "true" | "false" | "var" | "dynamic" | "using" | "checked" | "unchecked" | "stackalloc" | "fixed" | "lock" | "is" | "as" | "in" | "out" | "ref" | "params" | "enum" | "struct" | "class" | "interface" | "record" | "delegate" | "event" | "partial" | "sealed" | "abstract" | "virtual" | "override" | "new" | "static" | "readonly" | "const" | "volatile" | "unsafe" | "extern" | "internal" | "protected" | "private" | "public" | "File" | "Path" | "Directory" | "Environment" | "Regex" | "Linq" | "Enumerable" | "JsonSerializer" | "HttpClient"),
        "elixir" => matches!(word, "IO" | "puts" | "inspect" | "String" | "List" | "Map" | "Enum" | "Stream" | "Kernel" | "is_atom" | "is_binary" | "is_boolean" | "is_float" | "is_function" | "is_integer" | "is_list" | "is_map" | "is_number" | "is_pid" | "is_port" | "is_reference" | "is_tuple" | "elem" | "hd" | "tl" | "length" | "tuple_size" | "map_size" | "div" | "rem" | "abs" | "round" | "trunc" | "floor" | "ceil" | "min" | "max" | "inspect" | "self" | "send" | "receive" | "spawn" | "spawn_link" | "spawn_monitor" | "Process" | "Agent" | "GenServer" | "Task" | "Supervisor" | "Registry" | "pid" | "make_ref" | "node" | "Node" | "Application" | "Code" | "Mix" | "Logger" | "dbg" | "sigil_r" | "sigil_R" | "sigil_w" | "sigil_W"),
        "zig" => matches!(word, "std" | "builtin" | "mem" | "fmt" | "log" | "debug" | "testing" | "math" | "ArrayList" | "AutoHashMap" | "MultiArrayList" | "StringHashMap" | "PriorityQueue" | "Fifo" | "Stack" | "Allocator" | "GeneralPurposeAllocator" | "ArenaAllocator" | "FixedBufferAllocator" | "Thread" | "Mutex" | "RwLock" | "Semaphore" | "Atomic" | "fs" | "cwd" | "Dir" | "File" | "open" | "create" | "read" | "write" | "seek" | "print" | "println" | "panic" | "unreachable" | "compileError" | "compileLog" | "import" | "cImport" | "export" | "extern"),
        "html" => matches!(word, "document" | "window" | "console" | "Math" | "Date" | "Array" | "Object" | "String" | "Number" | "JSON" | "fetch" | "localStorage" | "sessionStorage" | "navigator" | "history" | "location" | "setTimeout" | "setInterval" | "requestAnimationFrame" | "addEventListener" | "querySelector" | "getElementById" | "createElement" | "innerHTML" | "textContent" | "classList" | "style" | "appendChild" | "removeChild" | "body" | "head"),
        "css" => matches!(word, "red" | "blue" | "green" | "white" | "black" | "transparent" | "currentColor" | "inherit" | "initial" | "unset" | "revert" | "auto" | "none" | "hidden" | "visible" | "scroll" | "fixed" | "absolute" | "relative" | "sticky" | "static" | "block" | "inline" | "inline-block" | "flex" | "grid" | "table" | "column" | "row" | "wrap" | "nowrap" | "center" | "start" | "end" | "space-between" | "space-around" | "baseline" | "stretch" | "cover" | "contain" | "repeat" | "no-repeat" | "bold" | "normal" | "italic" | "underline" | "solid" | "dashed" | "dotted"),
        "clojure" => matches!(word, "ns" | "refer" | "require" | "use" | "import" | "def" | "defn" | "defmacro" | "fn" | "let" | "if" | "cond" | "case" | "do" | "loop" | "recur" | "map" | "filter" | "reduce" | "comp" | "partial" | "apply" | "print" | "println" | "prn" | "str" | "symbol" | "keyword" | "vector" | "list" | "count" | "first" | "rest" | "next" | "last" | "cons" | "conj" | "assoc" | "dissoc" | "get" | "merge" | "keys" | "vals" | "into" | "empty" | "not" | "and" | "or" | "nil?" | "some?" | "every?" | "inc" | "dec" | "zero?" | "pos?" | "neg?" | "max" | "min" | "rand"),
        "erlang" => matches!(word, "spawn" | "spawn_link" | "spawn_monitor" | "send" | "receive" | "self" | "register" | "unregister" | "whereis" | "registered" | "exit" | "link" | "unlink" | "monitor" | "demonitor" | "apply" | "erlang" | "lists" | "maps" | "sets" | "dict" | "proplists" | "io" | "format" | "fwrite" | "read" | "file" | "open" | "close" | "read_file" | "write_file" | "delete" | "rename" | "make_ref" | "now" | "time" | "date" | "length" | "size" | "tuple_size" | "map_size" | "hd" | "tl" | "abs" | "round" | "trunc" | "element" | "setelement" | "binary_to_list" | "list_to_binary" | "atom_to_list" | "list_to_atom" | "integer_to_list" | "list_to_integer" | "float_to_list" | "list_to_float" | "is_atom" | "is_binary" | "is_boolean" | "is_float" | "is_function" | "is_integer" | "is_list" | "is_map" | "is_number" | "is_pid" | "is_port" | "is_record" | "is_reference" | "is_tuple" | "true" | "false" | "ok" | "error" | "undefined" | "self"),
        "diff" => matches!(word, "diff" | "index" | "new" | "deleted" | "modified" | "rename" | "copy"),
        "fsharp" => matches!(word, "printf" | "printfn" | "sprintf" | "fprintf" | "failwith" | "failwithf" | "invalidArg" | "invalidOp" | "nullArg" | "raise" | "reraise" | "try" | "with" | "finally" | "async" | "task" | "seq" | "query" | "List" | "Array" | "Seq" | "Map" | "Set" | "Option" | "Result" | "String" | "Math" | "Environment" | "Console" | "WriteLine" | "ReadLine" | "Path" | "File" | "Directory" | "DateTime" | "TimeSpan" | "Guid" | "BigInt" | "BigRational" | "Unit" | "box" | "hash" | "sizeof" | "typeof" | "nameof" | "id" | "fst" | "snd" | "ignore" | "not" | "ref" | "mutable" | "lazy"),
        "nim" => matches!(word, "echo" | "print" | "write" | "readLine" | "readFile" | "writeFile" | "open" | "close" | "parseInt" | "parseFloat" | "toInt" | "toFloat" | "toString" | "toStr" | "len" | "high" | "low" | "min" | "max" | "abs" | "succ" | "pred" | "inc" | "dec" | "new" | "newSeq" | "add" | "del" | "insert" | "delete" | "setLen" | "sort" | "map" | "filter" | "foldl" | "foldr" | "all" | "any" | "count" | "contains" | "items" | "pairs" | "fields" | "fieldPairs" | "System" | "math" | "strutils" | "sequtils" | "tables" | "sets" | "os" | "times" | "json" | "re" | "httpclient" | "async" | "asyncCheck" | "await" | "sleep" | "quit" | "assert" | "static" | "compileTime"),
        "crystal" => matches!(word, "puts" | "print" | "p" | "pp" | "raise" | "require" | "include" | "extend" | "private" | "protected" | "public" | "abstract" | "open_class" | "record" | "getter" | "setter" | "property" | "getter!" | "property!" | "forward" | "delegate" | "module" | "struct" | "class" | "lib" | "fun" | "macro" | "typeof" | "sizeof" | "instance_sizeof" | "pointerof" | "nil?" | "is_a?" | "responds_to?" | "as" | "as?" | "typeof" | "begin" | "end" | "ensure" | "rescue" | "yield" | "with" | "loop" | "sleep" | "spawn" | "send" | "nil" | "true" | "false" | "STDOUT" | "STDIN" | "STDERR" | "ARGC" | "ARGV"),
        "powershell" => matches!(word, "WriteHost" | "WriteOutput" | "WriteError" | "WriteWarning" | "WriteVerbose" | "WriteDebug" | "WriteInformation" | "WriteProgress" | "ReadHost" | "GetChildItem" | "GetItem" | "SetItem" | "GetContent" | "SetContent" | "AddContent" | "GetProcess" | "StopProcess" | "GetService" | "StartService" | "StopService" | "GetCommand" | "GetModule" | "ImportModule" | "GetHelp" | "GetMember" | "SelectObject" | "WhereObject" | "ForEachObject" | "SortObject" | "GroupObject" | "NewObject" | "RemoveItem" | "CopyItem" | "MoveItem" | "RenameItem" | "NewItem" | "TestPath" | "JoinPath" | "SplitPath" | "ConvertToJson" | "ConvertFromJson" | "FormatTable" | "FormatList" | "FormatWide" | "OutFile" | "OutHost" | "OutNull" | "OutString" | "CompareObject" | "StartJob" | "ReceiveJob" | "WaitJob" | "InvokeCommand" | "InvokeWebRequest" | "InvokeRestMethod" | "foreach" | "if" | "else" | "elseif" | "switch" | "try" | "catch" | "finally" | "throw" | "return" | "break" | "continue" | "exit" | "do" | "while" | "until" | "for" | "function" | "filter" | "param" | "begin" | "process" | "end" | "dynamicparam"),
        _ => false,
        }
    }

    fn render_autocomplete_popup(&self, f: &mut Frame, area: Rect, candidates: &[String], idx: isize) {
        let t = &self.theme;
        let is_slash = self.input.starts_with('/') && !self.input.contains(' ');
        let header = if is_slash { " Commands " } else { " @ Files & Refs " };

        let items: Vec<Line> = candidates
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let (icon, name, icon_color) = if let Some(r) = c.strip_prefix("ref:") {
                    (" ≡ ", r, t.secondary)
                } else if is_slash {
                    (" / ", c.as_str(), t.accent)
                } else {
                    // Check if it's a directory
                    let path = std::path::Path::new(c);
                    if path.is_dir() {
                        (" + ", c.as_str(), t.primary)
                    } else {
                        (" > ", c.as_str(), t.tool_call)
                    }
                };
                let selected = i as isize == idx;
                let style = if selected {
                    Style::default().fg(t.selected_list_item_text).bg(t.primary)
                } else {
                    Style::default().fg(t.text).bg(t.background_panel)
                };
                let dim = if selected { Modifier::empty() } else { Modifier::DIM };
                Line::from(vec![
                    Span::styled(icon, Style::default().fg(icon_color).add_modifier(dim)),
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
        let t = &self.theme;
        let border_color = if self.leader_mode { t.border_active } else { t.border };

        let outer_block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(t.background_element));

        let inner = outer_block.inner(area);

        f.render_widget(outer_block, area);

        // Show placeholder text if input is empty and not in leader mode
        if self.input.is_empty() && !self.leader_mode {
            let placeholder = Paragraph::new(Span::styled(
                " Type a message or / for commands...",
                Style::default().fg(t.text_dim).bg(t.background_element),
            ));
            f.render_widget(placeholder, inner);
        } else {
            let input = Paragraph::new(self.input.as_str())
                .style(Style::default().fg(t.text).bg(t.background_element))
                .wrap(Wrap { trim: true });
            f.render_widget(input, inner);
        }

        let cursor_pos = self.cursor.min(self.input.len()) as u16;
        f.set_cursor_position((inner.x + cursor_pos + 1, inner.y + 1));
    }

    // ── Dialog rendering ────────────────────────────────────

    fn render_dialog(&self, f: &mut Frame) {
        let dialog = match &self.dialog {
            Some(d) => d,
            None => return,
        };
        let t = &self.theme;
        let area = f.area();

        // Dimmed backdrop — fill full area with panel background to create depth contrast
        f.render_widget(Clear, area);
        let backdrop = Block::default()
            .style(Style::default().bg(t.background_element));
        f.render_widget(backdrop, area);

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
            "  Ctrl+C          Quit",
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
        let inner = Self::centered_rect(area, 60, 6);
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
                    Style::default().fg(t.selected_list_item_text).bg(t.primary)
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
        let dialog_area = Self::centered_rect(area, 60, height);
        let para = Paragraph::new(lines)
            .style(Style::default().bg(t.background_panel))
            .block(Block::default().borders(Borders::ALL)
                .title(format!(" {} ", title))
                .border_style(Style::default().fg(t.primary)));
        f.render_widget(Clear, dialog_area);
        f.render_widget(para, dialog_area);
    }

    fn dialog_area(area: Rect) -> Rect {
        Self::centered_rect(area, DIALOG_WIDTH, DIALOG_HEIGHT.min(area.height))
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
