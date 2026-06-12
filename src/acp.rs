use crate::config::Config;
use crate::session::Session;
use crate::session_store::SessionStore;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

#[derive(Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i64, message: String) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

pub struct AcpServer {
    config: Arc<Config>,
    store: Option<Arc<SessionStore>>,
}

impl AcpServer {
    pub fn new(config: Config, store: Option<SessionStore>) -> Self {
        Self {
            config: Arc::new(config),
            store: store.map(Arc::new),
        }
    }

    pub async fn run(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        let state = Arc::new(Mutex::new(AcpState {
            sessions: Vec::new(),
        }));

        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            let req: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    let resp = JsonRpcResponse::error(None, -32700, format!("Parse error: {}", e));
                    Self::write_response(&resp).await;
                    continue;
                }
            };

            let resp = self.handle_request(&req, &state).await;
            Self::write_response(&resp).await;
        }
        Ok(())
    }

    async fn write_response(resp: &JsonRpcResponse) {
        if let Ok(json) = serde_json::to_string(resp) {
            let mut stdout = tokio::io::stdout();
            let _ = stdout.write_all(json.as_bytes()).await;
            let _ = stdout.write_all(b"\n").await;
            let _ = stdout.flush().await;
        }
    }

    async fn handle_request(
        &self,
        req: &JsonRpcRequest,
        _state: &Arc<Mutex<AcpState>>,
    ) -> JsonRpcResponse {
        match req.method.as_str() {
            "chat" => {
                let prompt = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("prompt"))
                    .and_then(|p| p.as_str())
                    .unwrap_or("");
                let model = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("model"))
                    .and_then(|m| m.as_str());

                let mut session = match Session::new_from_config(
                    (*self.config).clone(),
                    model.map(|m| m.to_string()),
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        return JsonRpcResponse::error(
                            req.id.clone(),
                            -32603,
                            format!("Session creation failed: {}", e),
                        )
                    }
                };

                if let Err(e) = session.prompt(prompt).await {
                    return JsonRpcResponse::error(
                        req.id.clone(),
                        -32603,
                        format!("Chat failed: {}", e),
                    );
                }

                JsonRpcResponse::success(
                    req.id.clone(),
                    serde_json::json!({
                        "response": session.last_response.trim(),
                        "session_id": session.id,
                    }),
                )
            }
            "sessions/list" => {
                let sessions = match &self.store {
                    Some(store) => store.list_sessions(50).unwrap_or_default(),
                    None => Vec::new(),
                };
                JsonRpcResponse::success(req.id.clone(), serde_json::to_value(sessions).unwrap())
            }
            "sessions/get" => {
                let id = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("id"))
                    .and_then(|i| i.as_str())
                    .unwrap_or("");
                let store = match &self.store {
                    Some(s) => s,
                    None => {
                        return JsonRpcResponse::error(
                            req.id.clone(),
                            -32603,
                            "Store unavailable".into(),
                        )
                    }
                };
                let sessions = store.list_sessions(1000).unwrap_or_default();
                if let Some(s) = sessions.into_iter().find(|s| s.id == id) {
                    JsonRpcResponse::success(req.id.clone(), serde_json::to_value(s).unwrap())
                } else {
                    JsonRpcResponse::error(req.id.clone(), -32000, format!("Session {} not found", id))
                }
            }
            "ping" => {
                JsonRpcResponse::success(req.id.clone(), serde_json::json!("pong"))
            }
            _ => JsonRpcResponse::error(
                req.id.clone(),
                -32601,
                format!("Method '{}' not found", req.method),
            ),
        }
    }
}

struct AcpState {
    sessions: Vec<String>,
}
