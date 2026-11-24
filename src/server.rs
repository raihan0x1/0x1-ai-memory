use crate::config::SettingsPatch;
use crate::connectors::ConnectorManager;
use crate::db::MemoryDb;
use crate::distill::DistillHandle;
use crate::embed::Embedder;
use crate::hooks::{HookState, handle_hook};
use crate::llm::MemoryLlmClient;
use crate::mcp::handle_rpc;
use crate::models::{NewMemoryInput, SearchFilter, StreamEvent, UpdateMemoryInput};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use chrono::Utc;
use futures_util::Stream;
use rust_embed::Embed;
use serde::Deserialize;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};
use tower_http::cors::{Any, CorsLayer};

#[derive(Embed)]
#[folder = "ui/dist/"]
struct UiAssets;

#[derive(Clone)]
pub struct AppState {
    pub db: MemoryDb,
    pub embedder: Embedder,
    pub llm: MemoryLlmClient,
    pub distill: DistillHandle,
    pub connectors: ConnectorManager,
    pub events_tx: broadcast::Sender<StreamEvent>,
    pub hook_state: HookState,
    pub port: u16,
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health & MCP Streamable HTTP
        .route("/health", get(health))
        .route("/mcp", post(mcp_http_post).delete(mcp_http_delete))
        // Ultra-fast client hook endpoints
        .route("/v1/hooks/{client}", post(hook_endpoint))
        // REST API for Web UI & CLI
        .route("/api/stream", get(sse_stream))
        .route("/api/summary", get(api_summary))
        .route("/api/projects", get(api_list_projects))
        .route("/api/projects/{id}/privacy", patch(api_project_privacy))
        .route(
            "/api/memories",
            get(api_search_memories).post(api_create_memory),
        )
        .route(
            "/api/memories/{id}",
            get(api_get_memory)
                .patch(api_update_memory)
                .delete(api_delete_memory),
        )
        .route("/api/memories/{id}/timeline", get(api_memory_timeline))
        .route("/api/context-preview", get(api_context_preview))
        .route("/api/usage", get(api_usage_history))
        .route(
            "/api/settings",
            get(api_get_settings).patch(api_patch_settings),
        )
        .route("/api/engine/test", post(api_test_engine))
        .route("/api/connections", get(api_list_connections))
        .route("/api/connections/universal", get(api_universal_snippets))
        .route(
            "/api/connections/{client}/preview",
            get(api_preview_connect),
        )
        .route(
            "/api/connections/{client}/connect",
            post(api_connect_client),
        )
        .route(
            "/api/connections/{client}/disconnect",
            post(api_disconnect_client),
        )
        .route("/api/connections/{client}/ping", post(api_ping_client))
        // Embedded React SPA fallback
        .fallback(static_ui_handler)
        .layer(cors)
        .with_state(state)
}

async fn health(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "service": "0x1-ai-memory",
        "version": env!("CARGO_PKG_VERSION"),
        "port": st.port
    }))
}

async fn mcp_http_post(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let client_hint = headers
        .get("x-ox1-client")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
        });

    match handle_rpc(body, &st.db, &st.embedder, &st.events_tx, client_hint).await {
        Some(resp_json) => {
            let mut resp = Json(resp_json).into_response();
            resp.headers_mut().insert(
                header::HeaderName::from_static("mcp-session-id"),
                HeaderValue::from_static("ox1-local-session"),
            );
            resp
        }
        None => StatusCode::ACCEPTED.into_response(),
    }
}

async fn mcp_http_delete() -> StatusCode {
    StatusCode::OK
}

#[derive(Deserialize)]
struct HookQuery {
    event: Option<String>,
}

async fn hook_endpoint(
    State(st): State<AppState>,
    AxumPath(client): AxumPath<String>,
    Query(q): Query<HookQuery>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    let out = handle_hook(
        &client,
        q.event.as_deref(),
        payload,
        &st.db,
        &st.embedder,
        &st.distill,
        &st.events_tx,
        &st.hook_state,
    )
    .await;
    Json(out)
}

async fn sse_stream(
    State(st): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = st.events_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(ev) => {
            let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
            Some(Ok(Event::default().event(ev.kind).data(data)))
        }
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn api_summary(State(st): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let settings = st.db.get_settings().map_err(internal_err)?;
    let emb_status = st.embedder.status_label(&settings);
    let summary = st
        .db
        .usage_summary(&settings, emb_status)
        .map_err(internal_err)?;
    Ok(Json(serde_json::to_value(summary).unwrap_or_default()))
}

async fn api_list_projects(
    State(st): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let projects = st.db.list_projects().map_err(internal_err)?;
    Ok(Json(json!({ "projects": projects })))
}

#[derive(Deserialize)]
struct PrivacyBody {
    privacy_mode: String,
}

async fn api_project_privacy(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<PrivacyBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let updated = st
        .db
        .set_project_privacy(&id, &body.privacy_mode)
        .map_err(internal_err)?;
    Ok(Json(json!({ "updated": updated })))
}

async fn api_search_memories(
    State(st): State<AppState>,
    Query(filter): Query<SearchFilter>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let settings = st.db.get_settings().map_err(internal_err)?;
    let q_vec = match filter.query.as_deref() {
        Some(q) if !q.trim().is_empty() => st.embedder.embed_text(q, &settings).await,
        _ => None,
    };
    let hits = st
        .db
        .search_memories(&filter, q_vec.as_deref())
        .map_err(internal_err)?;
    Ok(Json(json!({ "count": hits.len(), "memories": hits })))
}

async fn api_create_memory(
    State(st): State<AppState>,
    Json(mut input): Json<NewMemoryInput>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if input.source_client.is_none() {
        input.source_client = Some("ui".into());
    }
    let settings = st.db.get_settings().map_err(internal_err)?;
    let embed_text = format!("{}\n{}\n{}", input.title, input.summary, input.content);
    let vec = st.embedder.embed_text(&embed_text, &settings).await;
    let mem = st
        .db
        .create_memory(input, vec.as_deref())
        .map_err(internal_err)?;
    let _ = st.events_tx.send(StreamEvent {
        kind: "memory_created".into(),
        timestamp: Utc::now().to_rfc3339(),
        payload: serde_json::to_value(&mem).unwrap_or_default(),
    });
    Ok(Json(json!({ "memory": mem })))
}

async fn api_get_memory(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mem = st
        .db
        .get_memories_by_ids(&[id])
        .map_err(internal_err)?
        .pop();
    match mem {
        Some(m) => Ok(Json(json!({ "memory": m }))),
        None => Err((StatusCode::NOT_FOUND, "Memory not found".into())),
    }
}

async fn api_update_memory(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(patch): Json<UpdateMemoryInput>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let settings = st.db.get_settings().map_err(internal_err)?;
    let needs_reembed = patch.title.is_some() || patch.summary.is_some() || patch.content.is_some();
    let updated = st
        .db
        .update_memory(&id, patch, None)
        .map_err(internal_err)?;
    match updated {
        Some(m) => {
            if needs_reembed {
                let text = format!("{}\n{}\n{}", m.title, m.summary, m.content);
                if let Some(vec) = st.embedder.embed_text(&text, &settings).await {
                    let _ = st.db.set_memory_embedding(&m.id, &vec);
                }
            }
            let _ = st.events_tx.send(StreamEvent {
                kind: "memory_updated".into(),
                timestamp: Utc::now().to_rfc3339(),
                payload: serde_json::to_value(&m).unwrap_or_default(),
            });
            Ok(Json(json!({ "memory": m })))
        }
        None => Err((StatusCode::NOT_FOUND, "Memory not found".into())),
    }
}

async fn api_delete_memory(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let deleted = st.db.delete_memory(&id).map_err(internal_err)?;
    if deleted {
        let _ = st.events_tx.send(StreamEvent {
            kind: "memory_deleted".into(),
            timestamp: Utc::now().to_rfc3339(),
            payload: json!({ "id": id }),
        });
    }
    Ok(Json(json!({ "deleted": deleted })))
}

async fn api_memory_timeline(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let items = st.db.memory_timeline(&id, 5, 5).map_err(internal_err)?;
    Ok(Json(json!({ "timeline": items })))
}

#[derive(Deserialize)]
struct ContextPreviewQuery {
    project_id: Option<String>,
}

async fn api_context_preview(
    State(st): State<AppState>,
    Query(q): Query<ContextPreviewQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let settings = st.db.get_settings().map_err(internal_err)?;
    let (briefing, count) = st
        .db
        .build_session_briefing(q.project_id.as_deref(), settings.session_start_token_budget)
        .map_err(internal_err)?;
    let approx_tokens = briefing.len().div_ceil(4);
    Ok(Json(json!({
        "briefing": briefing,
        "memory_count": count,
        "approx_tokens": approx_tokens,
        "token_budget": settings.session_start_token_budget
    })))
}

async fn api_usage_history(
    State(st): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let entries = st.db.recent_usage(50).map_err(internal_err)?;
    Ok(Json(json!({ "usage": entries })))
}

async fn api_get_settings(State(st): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let settings = st.db.get_settings().map_err(internal_err)?;
    Ok(Json(
        serde_json::to_value(settings.redacted()).unwrap_or_default(),
    ))
}

async fn api_patch_settings(
    State(st): State<AppState>,
    Json(patch): Json<SettingsPatch>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut settings = st.db.get_settings().map_err(internal_err)?;
    settings.apply_patch(patch);
    st.db.save_settings(&settings).map_err(internal_err)?;
    if settings.embedding_mode == "local_onnx" {
        st.embedder.warm_local_async();
    }
    Ok(Json(
        serde_json::to_value(settings.redacted()).unwrap_or_default(),
    ))
}

async fn api_test_engine(State(st): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let settings = st.db.get_settings().map_err(internal_err)?;
    let result = st.llm.test_connection(&settings).await;
    let _ = st.db.record_usage(
        &result.provider,
        &result.model,
        "test",
        result.input_tokens,
        result.output_tokens,
        0.0,
        None,
        if result.ok { "ok" } else { "error" },
        if result.ok {
            None
        } else {
            Some(&result.message)
        },
    );
    Ok(Json(serde_json::to_value(result).unwrap_or_default()))
}

async fn api_list_connections(
    State(st): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let conns = st.connectors.inspect_all(&st.db).map_err(internal_err)?;
    Ok(Json(json!({ "connections": conns })))
}

async fn api_universal_snippets(State(st): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(st.connectors.universal_snippets()).unwrap_or_default())
}

async fn api_preview_connect(
    State(st): State<AppState>,
    AxumPath(client): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let preview = st
        .connectors
        .preview_connect(&client)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(serde_json::to_value(preview).unwrap_or_default()))
}

async fn api_connect_client(
    State(st): State<AppState>,
    AxumPath(client): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let conn = st
        .connectors
        .connect(&client, &st.db)
        .map_err(internal_err)?;
    let _ = st.events_tx.send(StreamEvent {
        kind: "connection_changed".into(),
        timestamp: Utc::now().to_rfc3339(),
        payload: serde_json::to_value(&conn).unwrap_or_default(),
    });
    Ok(Json(json!({ "connection": conn })))
}

async fn api_disconnect_client(
    State(st): State<AppState>,
    AxumPath(client): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let conn = st
        .connectors
        .disconnect(&client, &st.db)
        .map_err(internal_err)?;
    let _ = st.events_tx.send(StreamEvent {
        kind: "connection_changed".into(),
        timestamp: Utc::now().to_rfc3339(),
        payload: serde_json::to_value(&conn).unwrap_or_default(),
    });
    Ok(Json(json!({ "connection": conn })))
}

async fn api_ping_client(
    State(st): State<AppState>,
    AxumPath(client): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _ = st.db.touch_client_activity(&client, false);
    let conns = st.connectors.inspect_all(&st.db).map_err(internal_err)?;
    let found = conns.into_iter().find(|c| c.client_id == client);
    Ok(Json(json!({
        "ok": true,
        "mcp_url": st.connectors.mcp_url(),
        "connection": found
    })))
}

async fn static_ui_handler(uri: Uri) -> Response {
    let raw_path = uri.path().trim_start_matches('/');
    let path = if raw_path.is_empty() {
        "index.html"
    } else {
        raw_path
    };

    if let Some(file) = UiAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return (
            [(header::CONTENT_TYPE, mime.as_ref().to_string())],
            file.data.into_owned(),
        )
            .into_response();
    }

    // SPA fallback to index.html for client-side routes (/memories, /connections, /engine)
    if let Some(index) = UiAssets::get("index.html") {
        return (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
            index.data.into_owned(),
        )
            .into_response();
    }

    (StatusCode::NOT_FOUND, "UI assets not built").into_response()
}

fn internal_err(e: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
