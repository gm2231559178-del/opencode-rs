use crate::llm::provider::{ContentPart, Message, Role};
use anyhow::{bail, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareRecord {
    pub id: String,
    pub session_id: String,
    pub secret: String,
    pub model: String,
    pub created_at: String,
}

pub struct SessionStore {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredSession {
    pub id: String,
    pub model: String,
    pub system_prompt: String,
    pub cwd: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

impl SessionStore {
    pub fn new() -> Result<Self> {
        let db_path = Self::db_path()?;
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                model TEXT NOT NULL DEFAULT '',
                system_prompt TEXT NOT NULL DEFAULT '',
                cwd TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                role TEXT NOT NULL,
                content_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
            CREATE TABLE IF NOT EXISTS shared_sessions (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                secret TEXT NOT NULL,
                model TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn share_session(&self, session_id: &str) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let share_id = uuid::Uuid::new_v4().to_string();
        let secret = uuid::Uuid::new_v4().to_string();
        let model: String = conn
            .query_row(
                "SELECT model FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap_or_default();
        conn.execute(
            "INSERT INTO shared_sessions (id, session_id, secret, model)
             VALUES (?1, ?2, ?3, ?4)",
            params![share_id, session_id, secret, model],
        )?;
        Ok(format!("{}\nSecret: {}", share_id, secret))
    }

    pub fn import_shared_session(&self, share_id: &str, secret: &str) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let result: Option<(String, String)> = conn
            .query_row(
                "SELECT session_id, secret FROM shared_sessions WHERE id = ?1",
                params![share_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let (session_id, stored_secret) = match result {
            Some(r) => r,
            None => bail!("Share '{}' not found.", share_id),
        };
        if stored_secret != secret {
            bail!("Invalid secret for share '{}'.", share_id);
        }
        let new_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO sessions (id, model, cwd)
             SELECT ?1, model, cwd FROM sessions WHERE id = ?2",
            params![new_id, session_id],
        )?;
        conn.execute(
            "INSERT INTO messages (session_id, role, content_json)
             SELECT ?1, role, content_json FROM messages WHERE session_id = ?2",
            params![new_id, session_id],
        )?;
        Ok(new_id)
    }

    pub fn list_shares(&self) -> Result<Vec<ShareRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, secret, model, created_at
             FROM shared_sessions ORDER BY created_at DESC",
        )?;
        let records = stmt
            .query_map([], |row| {
                Ok(ShareRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    secret: row.get(2)?,
                    model: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(records)
    }

    fn db_path() -> Result<PathBuf> {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("opencode-rs");
        Ok(base.join("sessions.db"))
    }

    pub fn save_session(
        &self,
        id: &str,
        model: &str,
        system_prompt: &str,
        cwd: &str,
        messages: &[Message],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, model, system_prompt, cwd, updated_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(id) DO UPDATE SET
                model = excluded.model,
                system_prompt = excluded.system_prompt,
                cwd = excluded.cwd,
                updated_at = datetime('now')",
            params![id, model, system_prompt, cwd],
        )?;

        conn.execute("DELETE FROM messages WHERE session_id = ?1", params![id])?;
        for msg in messages {
            let content_json = serde_json::to_string(&msg.content)?;
            let role_str = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            conn.execute(
                "INSERT INTO messages (session_id, role, content_json) VALUES (?1, ?2, ?3)",
                params![id, role_str, content_json],
            )?;
        }
        Ok(())
    }

    pub fn load_session(&self, id: &str) -> Result<Option<(String, String, String, Vec<Message>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT model, system_prompt, cwd FROM sessions WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        let row = match rows.next()? {
            Some(r) => r,
            None => return Ok(None),
        };
        let model: String = row.get(0)?;
        let system_prompt: String = row.get(1)?;
        let cwd: String = row.get(2)?;

        let mut msg_stmt = conn.prepare(
            "SELECT role, content_json FROM messages WHERE session_id = ?1 ORDER BY id",
        )?;
        let messages: Vec<Message> = msg_stmt
            .query_map(params![id], |row| {
                let role_str: String = row.get(0)?;
                let content_json: String = row.get(1)?;
                Ok((role_str, content_json))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(role_str, content_json)| {
                let role = match role_str.as_str() {
                    "system" => Role::System,
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    "tool" => Role::Tool,
                    _ => return None,
                };
                let content: Vec<ContentPart> = serde_json::from_str(&content_json).ok()?;
                Some(Message { role, content })
            })
            .collect();

        Ok(Some((model, system_prompt, cwd, messages)))
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<StoredSession>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.model, s.system_prompt, s.cwd, s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count
             FROM sessions s
             ORDER BY s.updated_at DESC
             LIMIT ?1",
        )?;
        let sessions = stmt
            .query_map(params![limit as i64], |row| {
                Ok(StoredSession {
                    id: row.get(0)?,
                    model: row.get(1)?,
                    system_prompt: row.get(2)?,
                    cwd: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    message_count: row.get::<_, i64>(6)? as usize,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(sessions)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM messages WHERE session_id = ?1", params![id])?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    }
}
