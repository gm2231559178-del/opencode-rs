use anyhow::{bail, Result};
use lsp_types::{
    ClientCapabilities, Diagnostic, InitializeParams,
    PublishDiagnosticsParams,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

pub struct LspServer {
    server_id: String,
    language_id: String,
    command: String,
    args: Vec<String>,
    connection: Arc<Mutex<LspConnection>>,
    pub diagnostics: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
}

struct LspConnection {
    child: Child,
    stdin: tokio::io::BufWriter<tokio::process::ChildStdin>,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
    initialized: bool,
    diagnostics: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
}

impl LspConnection {
    async fn connect(
        command: &str,
        args: &[String],
        diagnostics: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
    ) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        Ok(Self {
            child,
            stdin: tokio::io::BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            next_id: 1,
            initialized: false,
            diagnostics,
        })
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        loop {
            let mut response_line = String::new();
            self.stdout.read_line(&mut response_line).await?;
            if response_line.trim().is_empty() {
                continue;
            }
            let response: Value = serde_json::from_str(&response_line)?;

            if response.get("id").and_then(|i| i.as_u64()) == Some(id) {
                if let Some(error) = response.get("error") {
                    bail!("LSP error: {}", error);
                }
                return Ok(response["result"].clone());
            }

            if let Some(method) = response.get("method").and_then(|m| m.as_str()) {
                if method == "textDocument/publishDiagnostics" {
                    if let Some(params_val) = response.get("params") {
                        if let Ok(params) = serde_json::from_value::<PublishDiagnosticsParams>(params_val.clone()) {
                            let uri = params.uri.to_string();
                            let count = params.diagnostics.len();
                            let mut map = self.diagnostics.lock().await;
                            map.insert(uri, params.diagnostics);
                            tracing::info!("LSP: stored {} diagnostics", count);
                        }
                    }
                }
            }
        }
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&notification)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn initialize(&mut self, root_uri: &str) -> Result<()> {
        self.initialized = true;
        let capabilities = ClientCapabilities {
            text_document: Some(lsp_types::TextDocumentClientCapabilities {
                synchronization: Some(lsp_types::TextDocumentSyncClientCapabilities {
                    dynamic_registration: Some(true),
                    will_save: Some(true),
                    will_save_wait_until: Some(true),
                    did_save: Some(true),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.parse()?),
            capabilities,
            ..Default::default()
        };

        self.send_request("initialize", serde_json::to_value(params)?)
            .await?;

        self.send_notification("initialized", serde_json::json!({}))
            .await?;

        Ok(())
    }

    async fn open_document(&mut self, uri: &str, language_id: &str, text: &str) -> Result<()> {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": 1,
                "text": text,
            }
        });
        self.send_notification("textDocument/didOpen", params).await?;
        Ok(())
    }

    async fn change_document(&mut self, uri: &str, text: &str, version: i32) -> Result<()> {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "version": version,
            },
            "contentChanges": [{
                "text": text,
            }],
        });
        self.send_notification("textDocument/didChange", params)
            .await?;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        if self.initialized {
            let _ = self.send_request("shutdown", serde_json::json!(null)).await;
            self.send_notification("exit", serde_json::json!(null)).await?;
        }
        Ok(())
    }
}

impl LspServer {
    pub async fn connect(
        server_id: &str,
        command: &str,
        args: &[String],
        root_uri: &str,
        language_id: &str,
    ) -> Result<Self> {
        let diagnostics: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>> = Arc::new(Mutex::new(HashMap::new()));
        let mut conn = LspConnection::connect(command, args, diagnostics.clone()).await?;
        conn.initialize(root_uri).await?;

        Ok(Self {
            server_id: server_id.to_string(),
            language_id: language_id.to_string(),
            command: command.to_string(),
            args: args.to_vec(),
            connection: Arc::new(Mutex::new(conn)),
            diagnostics,
        })
    }

    pub async fn open_file(&self, file_path: &str) -> Result<()> {
        let content = tokio::fs::read_to_string(file_path).await?;
        let uri = format!("file://{}", file_path);
        let mut conn = self.connection.lock().await;
        conn.open_document(&uri, &self.language_id, &content).await?;
        Ok(())
    }

    pub async fn language_for_file(file_path: &str) -> Option<&'static str> {
        let ext = Path::new(file_path).extension()?.to_str()?;
        match ext {
            "rs" => Some("rust"),
            "ts" | "tsx" => Some("typescript"),
            "js" | "jsx" => Some("javascript"),
            "py" => Some("python"),
            "go" => Some("go"),
            "java" => Some("java"),
            "json" => Some("json"),
            "yaml" | "yml" => Some("yaml"),
            "md" => Some("markdown"),
            "css" | "scss" => Some("css"),
            "html" => Some("html"),
            _ => None,
        }
    }

    pub async fn lsp_command_for_file(file_path: &str) -> Option<(&'static str, Vec<String>)> {
        let ext = Path::new(file_path).extension()?.to_str()?;
        match ext {
            "rs" => Some(("rust-analyzer", vec![])),
            "ts" | "tsx" => Some(("typescript-language-server", vec!["--stdio".into()])),
            "js" | "jsx" => Some(("typescript-language-server", vec!["--stdio".into()])),
            "py" => Some(("pyright-langserver", vec!["--stdio".into()])),
            "go" => Some(("gopls", vec![])),
            _ => None,
        }
    }
}

pub struct LspManager {
    servers: Vec<LspServer>,
}

impl LspManager {
    pub fn new() -> Self {
        Self {
            servers: Vec::new(),
        }
    }

    pub async fn open_file(&mut self, file_path: &str) -> Result<Vec<Diagnostic>> {
        let language_id = match LspServer::language_for_file(file_path).await {
            Some(l) => l,
            None => return Ok(Vec::new()),
        };

        let server_id = format!("{}-{}", language_id, file_path);

        if !self.servers.iter().any(|s| s.server_id == server_id) {
            let root = std::env::current_dir()?;
            let root_uri = format!("file://{}", root.display());

            let (command, args) = match LspServer::lsp_command_for_file(file_path).await {
                Some(c) => c,
                None => return Ok(Vec::new()),
            };

            match LspServer::connect(&server_id, command, &args, &root_uri, language_id).await {
                Ok(server) => {
                    server.open_file(file_path).await?;
                    self.servers.push(server);
                }
                Err(e) => {
                    tracing::warn!("Failed to start LSP server for {}: {}", file_path, e);
                    return Ok(Vec::new());
                }
            }
        }

        let server = self.servers.iter().find(|s| s.server_id == server_id).unwrap();
        let diags = server.diagnostics.lock().await;
        let uri = format!("file://{}", std::path::Path::new(file_path).canonicalize().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| file_path.to_string()));
        Ok(diags.get(&uri).cloned().unwrap_or_default())
    }

    pub fn server_count(&self) -> usize {
        self.servers.len()
    }
}
