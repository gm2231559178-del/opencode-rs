use crate::llm::provider::{PermissionAction, StreamEvent};
use crate::session::Session;
use crate::session_store::SessionStore;
use anyhow::Result;
use arboard::Clipboard;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
}

#[derive(Clone)]
pub struct TuiMessage {
    pub role: String,
    pub content: String,
}

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
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;

        let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;

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

    fn poll_stream(&mut self) {
        if !self.streaming {
            return;
        }
        let mut done = false;
        if let Some(rx) = &mut self.stream_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    StreamEvent::Text { delta } => {
                        let needs_new = self.messages.last().map(|m| m.role != "assistant").unwrap_or(true);
                        if needs_new {
                            self.messages.push(TuiMessage {
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
                            role: "tool_call".to_string(),
                            content: format!("{} ({})\n{}", name, short, preview),
                        });
                    }
                    StreamEvent::PermissionRequest { request_id, tool_name, args } => {
                        let args_str = serde_json::to_string_pretty(&args).unwrap_or_default();
                        let preview: String = args_str.chars().take(200).collect();
                        self.messages.push(TuiMessage {
                            role: "tool_call".to_string(),
                            content: format!("{} (AWAITING APPROVAL)\n{}", tool_name, preview),
                        });
                        self.pending_perm = Some(request_id);
                    }
                    StreamEvent::ToolResult { name, output, .. } => {
                        let preview: String = output.chars().take(300).collect();
                        let truncated = preview.len() < output.len();
                        let content = if truncated {
                            format!("{} ({} bytes, showing first 300)\n{}", name, output.len(), preview)
                        } else {
                            format!("{} ({} bytes)\n{}", name, output.len(), preview)
                        };
                        self.messages.push(TuiMessage {
                            role: "tool_result".to_string(),
                            content,
                        });
                    }
                    StreamEvent::Done { response } => {
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
                                    role: "assistant".to_string(),
                                    content: response.trim().to_string(),
                                });
                            }
                        }
                        self.pending_response.clear();
                        done = true;
                    }
                    StreamEvent::Error { message } => {
                        let updated = self.messages.iter_mut().rev().find(|m| m.role == "assistant");
                        if let Some(msg) = updated {
                            msg.content = format!("Error: {}", message);
                        } else {
                            self.messages.push(TuiMessage {
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

    fn handle_slash(&mut self, cmd: &str) {
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
            "/diff" => {
                match self.session.try_lock() {
                    Ok(s) => s.show_diff(),
                    Err(_) => "Session busy, try again.".to_string(),
                }
            }
            "/help" => "Available commands:\n  /help          - Show this help\n  /plan          - Toggle plan mode (read-only)\n  /compact       - Compact conversation history\n  /diff          - Show diff of last file edit\n  /new           - Clear session\n  /model         - Show current model\n  /model <name>  - Switch model (e.g. /model openai/gpt-4o)\n  /agent         - Show available agents\n  /agent <name>  - Switch agent\n  /sessions      - List saved sessions\n  /session load <id>  - Load a saved session\n  /session fork       - Fork current session\n  /session delete <id> - Delete a session\n  /undo          - Undo last file change\n  /exit          - Quit OpenCode".to_string(),
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
            "/exit" | "/quit" | "/q" => {
                self.quit = true;
                String::new()
            }
            _ => format!("Unknown command: {}\nType /help for available commands.", cmd),
        };
        if !response.is_empty() {
            self.messages.push(TuiMessage {
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
                                role: "assistant".to_string(),
                                content: "Last response copied to clipboard.".to_string(),
                            });
                        } else {
                            self.messages.push(TuiMessage {
                                role: "assistant".to_string(),
                                content: "Failed to copy to clipboard.".to_string(),
                            });
                        }
                    }
                    Err(_) => {
                        self.messages.push(TuiMessage {
                            role: "assistant".to_string(),
                            content: "Clipboard not available.".to_string(),
                        });
                    }
                }
            }
            _ => {
                self.messages.push(TuiMessage {
                    role: "assistant".to_string(),
                    content: "No response to copy.".to_string(),
                });
            }
        }
    }

    fn trigger_autocomplete(&mut self) {
        // Find the last @ in input text after cursor
        let before_cursor = &self.input[..self.cursor];
        let at_pos = before_cursor.rfind('@');
        match at_pos {
            Some(pos) => {
                let query = before_cursor[pos + 1..].to_string();
                // Search for files matching query
                let pattern = if query.is_empty() {
                    "*".to_string()
                } else {
                    format!("*{}*", query)
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
                // Sort by proximity to query
                if !query.is_empty() {
                    candidates.sort_by_key(|c| c.to_lowercase().find(&query.to_lowercase()));
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
        if let Some(at_pos) = before_cursor.rfind('@') {
            // Replace from @ to cursor with the selected file path
            let after_cursor = &self.input[self.cursor..];
            let replacement = format!("{} ", selected);
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
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
            }
            KeyCode::Char('q') if self.input.is_empty() => {
                self.quit = true;
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
                        role: "user".to_string(),
                        content: msg.clone(),
                    });
                    self.scroll = 0;

                    if msg.starts_with('/') {
                        self.handle_slash(&msg);
                        return Ok(());
                    }

                    self.prompt_count += 1;
                    self.messages.push(TuiMessage {
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
                if c == '@' {
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

    fn render(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1), Constraint::Length(3)])
            .split(f.area());

        self.render_messages(f, chunks[0]);
        self.render_status(f, chunks[1]);
        self.render_input(f, chunks[2]);
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let status = if self.streaming { "streaming" } else { "idle" };
        let mode_tag = if self.plan_mode {
            Span::styled(
                " PLAN ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        };
        let left = Span::styled(
            format!(" {} ", self.model_name),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        );
        let right = Span::styled(
            format!(" prompts:{} | {} ", self.prompt_count, status),
            Style::default().fg(if self.streaming { Color::Green } else { Color::DarkGray }),
        );
        let mut spans = vec![left, Span::raw(" │ "), right];
        if self.plan_mode {
            spans.push(Span::raw(" │ "));
            spans.push(mode_tag);
        }
        let line = Line::from(spans);
        let block = Block::default().borders(Borders::TOP);
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(ratatui::widgets::Paragraph::new(line), inner);
    }

    fn render_messages(&self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .messages
            .iter()
            .rev()
            .skip(self.scroll)
            .map(|m| {
                let style = match m.role.as_str() {
                    "user" => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    "assistant" => Style::default().fg(Color::Green),
                    "tool_call" => Style::default().fg(Color::Yellow).add_modifier(Modifier::DIM),
                    "tool_result" => Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                    _ => Style::default(),
                };
                let label = match m.role.as_str() {
                    "tool_call" => "tool".to_string(),
                    "tool_result" => "result".to_string(),
                    r => r.to_string(),
                };
                let header = Span::styled(format!("{}> ", label), style);
                let content = textwrap::fill(&m.content, area.width as usize - 4);
                let lines: Vec<Line> = std::iter::once(Line::from(vec![header]))
                    .chain(content.lines().map(|l| Line::from(Span::raw(format!("  {}", l)))))
                    .chain(std::iter::once(Line::from("")))
                    .collect();
                ListItem::new(lines)
            })
            .collect();

        let messages = List::new(items)
            .block(Block::default().borders(Borders::TOP).title(" Chat "));

        f.render_widget(messages, area);
    }

    fn render_input(&self, f: &mut Frame, area: Rect) {
        let title = if self.pending_perm.is_some() {
            " Approve? (y=allow / n=deny) ".to_string()
        } else if !self.autocomplete_candidates.is_empty() {
            let idx = self.autocomplete_idx.max(0) as usize;
            let total = self.autocomplete_candidates.len();
            let preview = if idx < total {
                &self.autocomplete_candidates[idx]
            } else {
                ""
            };
            format!(" @ files ({}/{}) {} ", idx + 1, total, preview)
        } else {
            format!(
                " Input {}",
                if self.input_history.is_empty() {
                    ""
                } else {
                    "(↑↓ history)"
                }
            )
        };
        let input = Paragraph::new(self.input.as_str())
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: true });

        f.render_widget(input, area);

        let cursor_pos = self.input.len() as u16;
        f.set_cursor_position((area.x + cursor_pos + 1, area.y + 1));
    }
}
