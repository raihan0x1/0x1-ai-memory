use crate::config::{SettingsPatch, is_localhost_url};
use crate::connectors::ConnectorManager;
use crate::daemon::AutoStartManager;
use crate::db::MemoryDb;
use crate::distill::DistillHandle;
use crate::embed::Embedder;
use crate::hooks::{HookState, handle_hook};
use crate::llm::{ConnectionTestResult, MemoryLlmClient};
use crate::mcp::handle_rpc;
use crate::models::{NewMemoryInput, SearchFilter, StreamEvent, UpdateMemoryInput};
use crate::updater::UpdateManager;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxumPath, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    middleware::{self, Next},
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
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tower_http::cors::{AllowOrigin, CorsLayer};

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
    pub autostart: AutoStartManager,
    pub updater: Arc<UpdateManager>,
    pub events_tx: broadcast::Sender<StreamEvent>,
    pub shutdown_tx: watch::Sender<bool>,
    pub hook_state: HookState,
    pub port: u16,
    pub data_dir: PathBuf,
    pub log_path: PathBuf,
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _req_parts| {
            origin.to_str().map(is_localhost_url).unwrap_or(false)
        }))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::HeaderName::from_static("mcp-session-id"),
            header::HeaderName::from_static("x-ox1-client"),
        ]);

    Router::new()
        // Health & MCP Streamable HTTP
        .route("/health", get(health))
        .route("/mcp", post(mcp_http_post).delete(mcp_http_delete))
        // Ultra-fast client hook endpoints
        .route("/v1/hooks/{client}", post(hook_endpoint))
        // REST API for Web UI & CLI
        .route("/api/stream", get(sse_stream))
        .route("/api/summary", get(api_summary))
        .route("/api/sessions", get(api_list_sessions))
        .route("/api/sessions/{id}/distill", post(api_distill_session))
        .route("/api/raw-events", get(api_list_raw_events))
        .route("/api/distill/flush", post(api_distill_flush))
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
        .route("/api/memories/{id}/context", get(api_memory_context))
        .route("/api/graph", get(api_memory_graph))
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
        // Daemon status, opt-in Auto-Start, and clean stop (all localhost-guarded)
        .route("/api/daemon", get(api_daemon_info))
        .route("/api/daemon/stop", post(api_daemon_stop))
        .route("/api/daemon/autostart", post(api_daemon_autostart))
        // Daemon self-update and post-update "What's new" (all localhost-guarded)
        .route("/api/updates/status", get(api_updates_status))
        .route("/api/updates/check", post(api_updates_check))
        .route("/api/updates/apply", post(api_updates_apply))
        .route("/api/updates/dismiss", post(api_updates_dismiss))
        .route(
            "/api/updates/ack-whats-new",
            post(api_updates_ack_whats_new),
        )
        .route(
            "/api/updates/fetch-whats-new",
            post(api_updates_fetch_whats_new),
        )
        // Embedded React SPA fallback
        .fallback(static_ui_handler)
        .layer(middleware::from_fn(localhost_guard))
        .layer(cors)
        .with_state(state)
}

/// Blocks cross-origin browser requests and DNS-rebinding Host headers on this localhost-only daemon.
async fn localhost_guard(req: Request, next: Next) -> Response {
    if let Some(host) = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        && !is_localhost_url(host)
    {
        return (StatusCode::FORBIDDEN, "Non-localhost Host header rejected").into_response();
    }
    if let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        && !is_localhost_url(origin)
    {
        return (
            StatusCode::FORBIDDEN,
            "Cross-origin request rejected (localhost only)",
        )
            .into_response();
    }
    next.run(req).await
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
    let shutdown_rx = st.shutdown_tx.subscribe();
    // Terminate SSE immediately on shutdown so graceful server exit never hangs with open browser tabs.
    let stream = futures_util::stream::unfold((rx, shutdown_rx), |(mut rx, mut stop)| async move {
        if *stop.borrow() {
            return None;
        }
        loop {
            tokio::select! {
                biased;
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        return None;
                    }
                }
                msg = rx.recv() => {
                    match msg {
                        Ok(ev) => {
                            let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                            return Some((Ok(Event::default().event(ev.kind).data(data)), (rx, stop)));
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            }
        }
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
    let q_vec = memory_query_embedding(&st, &filter).await?;
    let limit = filter.limit.unwrap_or(20).clamp(1, 250);
    let offset = filter.offset.unwrap_or(0);
    let (hits, total) = st
        .db
        .search_memories_paged(&filter, q_vec.as_deref())
        .map_err(internal_err)?;
    Ok(Json(json!({
        "count": hits.len(),
        "total": total,
        "limit": limit,
        "offset": offset,
        "memories": hits,
    })))
}

async fn memory_query_embedding(
    st: &AppState,
    filter: &SearchFilter,
) -> Result<Option<Vec<f32>>, (StatusCode, String)> {
    let Some(query) = filter.query.as_deref().filter(|q| !q.trim().is_empty()) else {
        return Ok(None);
    };
    let settings = st.db.get_settings().map_err(internal_err)?;
    let allow_ext = !st.db.is_project_local_only(filter.project_id.as_deref());
    Ok(st
        .embedder
        .embed_text(query, &settings, Some(&st.db), allow_ext)
        .await)
}

async fn api_create_memory(
    State(st): State<AppState>,
    Json(mut input): Json<NewMemoryInput>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if input.source_client.is_none() {
        input.source_client = Some("ui".into());
    }
    let settings = st.db.get_settings().map_err(internal_err)?;
    let allow_ext = !st.db.is_project_local_only(input.project_id.as_deref());
    let embed_text = format!("{}\n{}\n{}", input.title, input.summary, input.content);
    let vec = st
        .embedder
        .embed_text(&embed_text, &settings, Some(&st.db), allow_ext)
        .await;
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
                let allow_ext = !st.db.is_project_local_only(m.project_id.as_deref());
                let text = format!("{}\n{}\n{}", m.title, m.summary, m.content);
                if let Some(vec) = st
                    .embedder
                    .embed_text(&text, &settings, Some(&st.db), allow_ext)
                    .await
                {
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

async fn api_memory_context(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    match st.db.memory_context(&id).map_err(internal_err)? {
        Some(ctx) => Ok(Json(serde_json::to_value(ctx).unwrap_or_default())),
        None => Err((StatusCode::NOT_FOUND, "Memory not found".into())),
    }
}

async fn api_memory_graph(
    State(st): State<AppState>,
    Query(filter): Query<SearchFilter>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let q_vec = memory_query_embedding(&st, &filter).await?;
    let limit = filter.limit.unwrap_or(180);
    let graph = st
        .db
        .build_memory_graph(&filter, limit, q_vec.as_deref())
        .map_err(internal_err)?;
    Ok(Json(serde_json::to_value(graph).unwrap_or_default()))
}

#[derive(Deserialize)]
struct SessionsQuery {
    project_id: Option<String>,
    client: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn api_list_sessions(
    State(st): State<AppState>,
    Query(q): Query<SessionsQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let limit = q.limit.unwrap_or(12).clamp(1, 100);
    let offset = q.offset.unwrap_or(0);
    let (sessions, total) = st
        .db
        .list_sessions_paged(
            q.project_id.as_deref().filter(|s| !s.is_empty()),
            q.client.as_deref().filter(|s| !s.is_empty()),
            limit,
            offset,
        )
        .map_err(internal_err)?;
    Ok(Json(json!({
        "sessions": sessions,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

#[derive(Deserialize)]
struct RawEventsQuery {
    project_id: Option<String>,
    client: Option<String>,
    kind: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn api_list_raw_events(
    State(st): State<AppState>,
    Query(q): Query<RawEventsQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0);
    let (events, total) = st
        .db
        .recent_raw_events_paged(
            q.project_id.as_deref().filter(|s| !s.is_empty()),
            q.client.as_deref().filter(|s| !s.is_empty()),
            q.kind.as_deref().filter(|s| !s.is_empty()),
            limit,
            offset,
        )
        .map_err(internal_err)?;
    Ok(Json(json!({
        "events": events,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

async fn api_distill_session(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Json<Value> {
    st.distill.queue_session(id.clone());
    Json(json!({ "queued": true, "session_id": id }))
}

async fn api_distill_flush(
    State(st): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_ids = st
        .db
        .sessions_with_any_pending_events(25)
        .map_err(internal_err)?;
    for sid in &session_ids {
        st.distill.queue_session(sid.clone());
    }
    Ok(Json(json!({
        "queued_sessions": session_ids.len(),
        "session_ids": session_ids
    })))
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
    let (detail, _) = st
        .db
        .build_session_briefing_detailed(
            q.project_id.as_deref().filter(|s| !s.is_empty()),
            settings.session_start_token_budget,
        )
        .map_err(internal_err)?;
    Ok(Json(serde_json::to_value(detail).unwrap_or_default()))
}

#[derive(Deserialize)]
struct UsageQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn api_usage_history(
    State(st): State<AppState>,
    Query(q): Query<UsageQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0);
    let (entries, total) = st
        .db
        .recent_usage_paged(limit, offset)
        .map_err(internal_err)?;
    Ok(Json(json!({
        "usage": entries,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
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

async fn api_test_engine(
    State(st): State<AppState>,
    body: Bytes,
) -> Result<Json<ConnectionTestResult>, (StatusCode, String)> {
    let saved = st.db.get_settings().map_err(internal_err)?;
    let mut settings = saved.clone();
    if !body.iter().all(|b| b.is_ascii_whitespace()) {
        let patch: SettingsPatch = serde_json::from_slice(&body).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid test settings payload: {e}"),
            )
        })?;
        settings.apply_patch(patch);
    }
    let result = if settings.provider != "none"
        && st
            .db
            .usage_summary(&saved, st.embedder.status_label(&saved))
            .map_err(internal_err)?
            .budget_exhausted
    {
        ConnectionTestResult {
            ok: false,
            status: "budget_blocked",
            provider: settings.provider,
            model: settings.model,
            latency_ms: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_usd: 0.0,
            message: "Today's saved token or USD limit was reached. Wait for the UTC reset or update the limit.".into(),
        }
    } else {
        st.llm.test_connection(&settings).await
    };
    st.db
        .record_usage(
            &result.provider,
            &result.model,
            "test",
            result.input_tokens,
            result.output_tokens,
            result.estimated_cost_usd,
            None,
            result.status,
            (!result.ok).then_some(result.message.as_str()),
        )
        .map_err(internal_err)?;
    Ok(Json(result))
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
    // Run an actual MCP ping RPC probe + inspect client config without faking client heartbeats.
    let rpc = handle_rpc(
        json!({ "jsonrpc": "2.0", "id": "ping", "method": "ping" }),
        &st.db,
        &st.embedder,
        &st.events_tx,
        None,
    )
    .await;
    let mcp_ok = rpc.as_ref().and_then(|v| v.get("result")).is_some();
    let conns = st.connectors.inspect_all(&st.db).map_err(internal_err)?;
    let found = conns.into_iter().find(|c| c.client_id == client);
    let config_connected = found.as_ref().map(|c| c.connected).unwrap_or(false);
    Ok(Json(json!({
        "ok": mcp_ok && config_connected,
        "mcp_ok": mcp_ok,
        "config_connected": config_connected,
        "mcp_url": st.connectors.mcp_url(),
        "connection": found
    })))
}

async fn api_daemon_info(State(st): State<AppState>) -> Json<Value> {
    let autostart = st.autostart.status();
    Json(json!({
        "running": true,
        "pid": std::process::id(),
        "port": st.port,
        "version": env!("CARGO_PKG_VERSION"),
        "mcp_url": st.connectors.mcp_url(),
        "data_dir": st.data_dir.display().to_string(),
        "log_path": st.log_path.display().to_string(),
        "autostart": autostart,
    }))
}

async fn api_daemon_stop(State(st): State<AppState>) -> Json<Value> {
    let tx = st.shutdown_tx.clone();
    tokio::spawn(async move {
        // Brief delay lets the HTTP 200 JSON response flush before tearing down listeners and SSE.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = tx.send(true);
    });
    Json(json!({ "stopping": true }))
}

#[derive(Deserialize)]
struct AutoStartBody {
    enabled: bool,
}

async fn api_daemon_autostart(
    State(st): State<AppState>,
    Json(body): Json<AutoStartBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status = st
        .autostart
        .set_enabled(body.enabled)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(serde_json::to_value(status).unwrap_or_default()))
}

async fn api_updates_status(State(st): State<AppState>) -> Json<Value> {
    let status = st.updater.status().await;
    Json(serde_json::to_value(status).unwrap_or_default())
}

async fn api_updates_check(State(st): State<AppState>) -> Json<Value> {
    let status = st.updater.check_for_updates(true).await;
    Json(serde_json::to_value(status).unwrap_or_default())
}

async fn api_updates_apply(
    State(st): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status = st.updater.start_self_update().await?;
    Ok(Json(serde_json::to_value(status).unwrap_or_default()))
}

#[derive(Deserialize)]
struct DismissUpdateBody {
    version: String,
}

async fn api_updates_dismiss(
    State(st): State<AppState>,
    Json(body): Json<DismissUpdateBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status = st
        .updater
        .dismiss_version(&body.version)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::to_value(status).unwrap_or_default()))
}

async fn api_updates_ack_whats_new(
    State(st): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status = st.updater.ack_whats_new().await.map_err(internal_err)?;
    Ok(Json(serde_json::to_value(status).unwrap_or_default()))
}

async fn api_updates_fetch_whats_new(
    State(st): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status = st
        .updater
        .fetch_whats_new_for_current()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(serde_json::to_value(status).unwrap_or_default()))
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
        let cache_control = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return (
            [
                (header::CONTENT_TYPE, mime.as_ref().to_string()),
                (header::CACHE_CONTROL, cache_control.to_string()),
            ],
            file.data.into_owned(),
        )
            .into_response();
    }

    if path.starts_with("assets/") {
        return (StatusCode::NOT_FOUND, "Asset not found").into_response();
    }

    // SPA fallback to index.html for client-side routes (/memories, /connections, /engine)
    if let Some(index) = UiAssets::get("index.html") {
        return (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            index.data.into_owned(),
        )
            .into_response();
    }

    (StatusCode::NOT_FOUND, "UI assets not built").into_response()
}

fn internal_err(e: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
