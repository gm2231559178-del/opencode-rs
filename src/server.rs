use crate::config::Config;
use crate::session::Session;
use crate::session_store::SessionStore;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{http::StatusCode, Json};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub store: Option<Arc<SessionStore>>,
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub prompt: String,
    pub model: Option<String>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

pub async fn run_server(config: Config, store: Option<SessionStore>, port: u16) {
    let _mdns = crate::mdns::register_service("opencode", port);

    let state = AppState {
        config: Arc::new(config),
        store: store.map(Arc::new),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/chat", post(chat))
        .route("/sessions", get(list_sessions))
        .route("/sessions/:id", get(get_session))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("HTTP server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let model = req.model.unwrap_or_else(|| {
        state
            .config
            .model
            .clone()
            .unwrap_or_else(|| "openai/gpt-4o".to_string())
    });

    let mut session = match Session::new_from_config((*state.config).clone(), Some(model)) {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    };

    if let Err(e) = session.prompt(&req.prompt).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response();
    }

    if let Some(store) = &state.store {
        let _ = store.save_session(
            &session.id,
            &session.model,
            &session.system_prompt,
            &session.cwd,
            &session.messages,
        );
    }

    (StatusCode::OK, Json(serde_json::json!({
        "response": session.last_response.trim(),
        "session_id": session.id,
    }))).into_response()
}

async fn list_sessions(
    State(state): State<AppState>,
) -> impl IntoResponse {
    match &state.store {
        Some(store) => match store.list_sessions(20) {
            Ok(sessions) => (StatusCode::OK, Json(serde_json::to_value(sessions).unwrap())).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
        },
        None => (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": "store unavailable"}))).into_response(),
    }
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let store = match &state.store {
        Some(s) => s,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": "store unavailable"}))).into_response(),
    };
    match store.list_sessions(1000) {
        Ok(sessions) => {
            if let Some(s) = sessions.into_iter().find(|s| s.id == id) {
                (StatusCode::OK, Json(serde_json::to_value(s).unwrap())).into_response()
            } else {
                (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": format!("session {} not found", id)}))).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}
