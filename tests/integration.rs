use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ox1_memory::connectors::ConnectorManager;
use ox1_memory::db::MemoryDb;
use ox1_memory::distill::{distill_session_once, spawn_distill_worker};
use ox1_memory::embed::Embedder;
use ox1_memory::hooks::HookState;
use ox1_memory::llm::MemoryLlmClient;
use ox1_memory::models::StreamEvent;
use ox1_memory::server::{AppState, build_router};
use serde_json::{Value, json};
use std::path::PathBuf;
use tempfile::tempdir;
use tokio::sync::broadcast;
use tower::ServiceExt;

fn test_state(tmp: &std::path::Path) -> AppState {
    let db = MemoryDb::open(&tmp.join("memory.db")).unwrap();
    db.save_settings(&ox1_memory::config::Settings::for_testing())
        .unwrap();
    let embedder = Embedder::new(tmp.join("models"));
    let llm = MemoryLlmClient::new();
    let (events_tx, _) = broadcast::channel::<StreamEvent>(32);
    let distill =
        spawn_distill_worker(db.clone(), embedder.clone(), llm.clone(), events_tx.clone());
    let connectors = ConnectorManager::new(
        tmp.join("home"),
        tmp.join("backups"),
        PathBuf::from("/usr/local/bin/0x1-memory"),
        10101,
    );
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let autostart = ox1_memory::daemon::AutoStartManager::for_testing(
        tmp.join("home"),
        PathBuf::from("/usr/local/bin/0x1-memory"),
        tmp.join("daemon.log"),
        10101,
        ox1_memory::daemon::AutoStartPlatform::MacOs,
    );
    AppState {
        db,
        embedder,
        llm,
        distill,
        connectors,
        autostart,
        events_tx,
        shutdown_tx,
        hook_state: HookState::default(),
        port: 10101,
        data_dir: tmp.to_path_buf(),
        log_path: tmp.join("daemon.log"),
    }
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn end_to_end_cross_client_mcp_hooks_ui_and_budget_guard() {
    let tmp = tempdir().unwrap();
    let repo_dir = tmp.path().join("demo-repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());

    // 1. Embedded React UI is served at /
    let ui_resp = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ui_resp.status(), StatusCode::OK);
    let html = String::from_utf8(
        ui_resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("0x1-ai-memory"));
    assert!(html.contains("/assets/"));

    // 2. Claude Code saves a memory via Streamable HTTP MCP
    let save_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/call",
                        "params": {
                            "name": "memory_save",
                            "arguments": {
                                "title": "Auth uses httpOnly refresh cookies",
                                "summary": "Replaced localStorage JWT with httpOnly cookie rotation",
                                "content": "See src/auth/session.rs for token refresh rotation.",
                                "type": "decision",
                                "scope": "project",
                                "project": repo_dir.to_string_lossy(),
                                "client": "claude-code"
                            }
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(save_resp.status(), StatusCode::OK);
    assert_eq!(
        save_resp.headers().get("mcp-session-id").unwrap(),
        "ox1-local-session"
    );

    // 3. Codex SessionStart hook immediately receives Claude Code's memory WITHOUT top-level systemMessage
    let codex_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hooks/codex?event=SessionStart")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "cwd": repo_dir.to_string_lossy(),
                        "session_id": "codex-sess-1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let codex_json = body_json(codex_resp).await;
    assert!(codex_json.get("systemMessage").is_none());
    assert!(
        codex_json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("httpOnly refresh cookies")
    );

    // 4. Antigravity SessionStart hook receives same memory in camelCase injectSteps format
    let agy_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hooks/antigravity?event=SessionStart")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "cwd": repo_dir.to_string_lossy(),
                        "conversationId": "agy-conv-1"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let agy_json = body_json(agy_resp).await;
    assert!(
        agy_json["injectSteps"][0]["systemMessage"]["systemMessage"]
            .as_str()
            .unwrap()
            .contains("httpOnly refresh cookies")
    );

    // 5. PostToolUse captures Bash/Edit at 0 tokens and scrubs secrets before SQLite
    let hook_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hooks/claude?event=PostToolUse")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "cwd": repo_dir.to_string_lossy(),
                        "session_id": "claude-sess-1",
                        "tool_name": "Bash",
                        "tool_input": { "command": "echo sk-or-v1-1234567890abcdef1234567890abcdef" },
                        "tool_response": "<private>do not store</private> ok"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hook_resp.status(), StatusCode::OK);

    // 6. Hard budget guard blocks distillation when daily cap is exhausted (ZERO silent fallback)
    let mut settings = state.db.get_settings().unwrap();
    settings.provider = "openrouter".into();
    settings.daily_token_cap = 1_000;
    state.db.save_settings(&settings).unwrap();
    state
        .db
        .record_usage(
            "openrouter",
            "test-model",
            "distill",
            800,
            300,
            0.01,
            None,
            "ok",
            None,
        )
        .unwrap();

    let sess = state
        .db
        .upsert_session(
            &ox1_memory::project::resolve_project(&repo_dir).id,
            "claude-code",
            "claude-sess-1",
            None,
        )
        .unwrap();
    // Verify scrubbed raw_event before distilling
    let pending = state.db.pending_events_for_session(&sess.id, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(!pending[0].payload_json.contains("sk-or-v1-"));
    assert!(!pending[0].payload_json.contains("do not store"));

    distill_session_once(
        &state.db,
        &state.embedder,
        &state.llm,
        &state.events_tx,
        &sess.id,
    )
    .await;
    let usage = state.db.recent_usage(5).unwrap();
    assert_eq!(usage[0].status, "budget_blocked");

    // 7. Regression: /api/memories/{id}/timeline does NOT deadlock
    let mems = state.db.search_memories(&Default::default(), None).unwrap();
    assert!(!mems.is_empty());
    let tl_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/memories/{}/timeline", mems[0].id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tl_resp.status(), StatusCode::OK);
    let tl_json = body_json(tl_resp).await;
    assert_eq!(tl_json["timeline"].as_array().unwrap().len(), 1);

    // 8. Regression: Non-localhost Origin and DNS-rebinding Host headers are blocked (403)
    let evil_origin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/memories")
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(evil_origin.status(), StatusCode::FORBIDDEN);

    let evil_host = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/memories")
                .header("host", "evil.example:10101")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(evil_host.status(), StatusCode::FORBIDDEN);

    let ok_origin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/memories")
                .header("host", "127.0.0.1:10101")
                .header("origin", "http://127.0.0.1:10101")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_origin.status(), StatusCode::OK);

    // 9. Regression: local_only blocks cli_* providers too (zero off-machine leakage)
    let proj_id = ox1_memory::project::resolve_project(&repo_dir).id;
    state
        .db
        .set_project_privacy(&proj_id, "local_only")
        .unwrap();
    settings.provider = "cli_claude".into();
    settings.daily_token_cap = 250_000;
    state.db.save_settings(&settings).unwrap();
    state
        .db
        .insert_raw_event(
            &sess.id,
            &proj_id,
            "claude-code",
            "prompt",
            None,
            r#"{"prompt":"secret repo"}"#,
        )
        .unwrap();
    let usage_before = state.db.recent_usage(20).unwrap().len();
    distill_session_once(
        &state.db,
        &state.embedder,
        &state.llm,
        &state.events_tx,
        &sess.id,
    )
    .await;
    assert_eq!(state.db.recent_usage(20).unwrap().len(), usage_before);
}

#[tokio::test]
async fn daemon_stop_closes_open_sse_and_exits_cleanly_plus_autostart_api() {
    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");

    let server_task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                while !*shutdown_rx.borrow() {
                    if shutdown_rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await
            .unwrap();
    });

    let client = reqwest::Client::new();

    // 1. Daemon metadata & opt-in Auto-Start toggle work over HTTP
    let info: Value = client
        .get(format!("{base}/api/daemon"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(info["running"], true);
    assert_eq!(info["autostart"]["supported"], true);
    assert_eq!(info["autostart"]["enabled"], false);

    let enabled: Value = client
        .post(format!("{base}/api/daemon/autostart"))
        .json(&json!({ "enabled": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(enabled["enabled"], true);
    let plist_path = tmp
        .path()
        .join("home/Library/LaunchAgents/com.0x1.memory.plist");
    assert!(plist_path.exists());

    // 2. Cross-origin POST /api/daemon/stop is blocked by localhost_guard
    let evil_stop = client
        .post(format!("{base}/api/daemon/stop"))
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(evil_stop.status(), reqwest::StatusCode::FORBIDDEN);

    // 3. Keep an active SSE connection open (exact repro of Ctrl-C hang!)
    let sse_resp = client
        .get(format!("{base}/api/stream"))
        .send()
        .await
        .unwrap();
    assert_eq!(sse_resp.status(), reqwest::StatusCode::OK);

    // 4. Localhost stop immediately tears down SSE and completes graceful shutdown (< 2s)
    let stop_resp = client
        .post(format!("{base}/api/daemon/stop"))
        .send()
        .await
        .unwrap();
    assert_eq!(stop_resp.status(), reqwest::StatusCode::OK);

    tokio::time::timeout(std::time::Duration::from_secs(2), server_task)
        .await
        .expect("Daemon hung on graceful shutdown with open SSE connection")
        .unwrap();

    drop(sse_resp);
}

#[tokio::test]
async fn engine_test_uses_unsaved_draft_without_mutating_saved_settings() {
    use axum::{Router, extract::Request as AxumRequest, routing::post};
    use std::sync::{Arc, Mutex};

    let captured = Arc::new(Mutex::new(Vec::<(Option<String>, Value)>::new()));
    let captured_srv = captured.clone();
    let mock_router = Router::new().route(
        "/v1/chat/completions",
        post(move |req: AxumRequest| {
            let captured_srv = captured_srv.clone();
            async move {
                let auth = req
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                let payload: Value = serde_json::from_slice(&bytes).unwrap_or_default();
                captured_srv.lock().unwrap().push((auth, payload));
                axum::Json(json!({
                    "choices": [{ "message": { "content": "{\"ok\":true}" } }],
                    "usage": { "prompt_tokens": 12, "completion_tokens": 4, "cost": 0.00001 }
                }))
            }
        }),
    );

    let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock_listener.local_addr().unwrap();
    let mock_task = tokio::spawn(async move {
        let _ = axum::serve(mock_listener, mock_router).await;
    });

    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());

    // 1. Legacy empty-body POST still tests saved Zero-LLM settings
    let empty_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/engine/test")
                .header("content-type", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(empty_resp.status(), StatusCode::OK);
    let empty_json = body_json(empty_resp).await;
    assert_eq!(empty_json["ok"], true);
    assert_eq!(empty_json["provider"], "none");

    // 2. Unsaved draft payload tests OpenRouter directly without mutating saved DB settings
    let draft_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/engine/test")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "provider": "openrouter",
                        "model": "deepseek/deepseek-v4.1-flash-1017",
                        "base_url": format!("http://{mock_addr}/v1"),
                        "api_key": "sk-test-unsaved"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(draft_resp.status(), StatusCode::OK);
    let draft_json = body_json(draft_resp).await;
    assert_eq!(draft_json["ok"], true);
    assert_eq!(draft_json["provider"], "openrouter");
    assert_eq!(draft_json["model"], "deepseek/deepseek-v4.1-flash-1017");
    assert_eq!(draft_json["input_tokens"], 12);
    assert_eq!(draft_json["output_tokens"], 4);

    // Saved DB settings remain untouched (still Zero-LLM with empty key)
    let saved_after = state.db.get_settings().unwrap();
    assert_eq!(saved_after.provider, "none");
    assert_eq!(saved_after.api_key, "");

    // 3. Omitting api_key in draft falls back to the stored api_key
    let mut stored = state.db.get_settings().unwrap();
    stored.api_key = "sk-stored-key".into();
    state.db.save_settings(&stored).unwrap();

    let stored_key_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/engine/test")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "provider": "openrouter",
                        "model": "google/gemini-2.5-flash-lite",
                        "base_url": format!("http://{mock_addr}/v1")
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stored_key_resp.status(), StatusCode::OK);
    {
        let calls = captured.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0.as_deref(), Some("Bearer sk-test-unsaved"));
        assert_eq!(calls[0].1["model"], "deepseek/deepseek-v4.1-flash-1017");
        assert_eq!(calls[1].0.as_deref(), Some("Bearer sk-stored-key"));
    }

    // 4. Malformed JSON returns 400 BAD_REQUEST
    let bad_resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/engine/test")
                .header("content-type", "application/json")
                .body(Body::from("{not-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_resp.status(), StatusCode::BAD_REQUEST);

    mock_task.abort();
}

#[tokio::test]
async fn sessions_raw_events_graph_context_and_recall_apis_work() {
    let tmp = tempdir().unwrap();
    let repo_dir = tmp.path().join("graph-repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());

    let proj = ox1_memory::project::resolve_project(&repo_dir);
    state.db.upsert_project(&proj).unwrap();

    // Hook prompt + PostToolUse (with null error)
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hooks/antigravity?event=PreInvocation")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "cwd": repo_dir.to_string_lossy(),
                        "conversationId": "conv-ui-1",
                        "lastUserInput": "Fix auth rotation"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hooks/antigravity?event=PostToolUse")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "cwd": repo_dir.to_string_lossy(),
                        "conversationId": "conv-ui-1",
                        "tool_name": "edit_file",
                        "tool_input": { "TargetFile": "src/auth.rs" },
                        "error": null
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Create two linked memories
    let m1 = state
        .db
        .create_memory(
            ox1_memory::models::NewMemoryInput {
                scope: Some("project".into()),
                project_id: Some(proj.id.clone()),
                session_id: None,
                source_client: Some("codex".into()),
                memory_type: "decision".into(),
                title: "Session auth v1".into(),
                summary: "Use signed cookies".into(),
                content: "Initial signed cookie implementation in src/auth.rs".into(),
                files: Some(vec!["src/auth.rs".into()]),
                tags: Some(vec!["auth".into()]),
                status: None,
                supersedes_id: None,
            },
            None,
        )
        .unwrap();

    let m2 = state
        .db
        .create_memory(
            ox1_memory::models::NewMemoryInput {
                scope: Some("project".into()),
                project_id: Some(proj.id.clone()),
                session_id: None,
                source_client: Some("claude-code".into()),
                memory_type: "decision".into(),
                title: "Session auth v2".into(),
                summary: "Rotate refresh tokens".into(),
                content: "Added refresh rotation in src/auth.rs".into(),
                files: Some(vec!["src/auth.rs".into()]),
                tags: Some(vec!["auth".into()]),
                status: Some("pinned".into()),
                supersedes_id: Some(m1.id.clone()),
            },
            None,
        )
        .unwrap();

    // SessionStart hook increments recall_count on m2
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/hooks/codex?event=SessionStart")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "cwd": repo_dir.to_string_lossy(),
                        "session_id": "codex-recall-check"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let sessions = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(!sessions["sessions"].as_array().unwrap().is_empty());

    let raw_events = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/raw-events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let evs = raw_events["events"].as_array().unwrap();
    assert!(evs.iter().any(|e| e["event_type"] == "tool_use"));
    assert!(!evs.iter().any(|e| e["event_type"] == "tool_error"));

    let graph = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/graph?project_id={}&include_inactive=true",
                        proj.id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert!(!graph["edges"].as_array().unwrap().is_empty());

    let ctx = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/memories/{}/context", m2.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(ctx["memory"]["recall_count"], 1);
    assert_eq!(ctx["lineage"].as_array().unwrap().len(), 2);
    assert!(!ctx["neighbors"].as_array().unwrap().is_empty());

    let preview = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/context-preview?project_id={}", proj.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(!preview["sections"].as_array().unwrap().is_empty());
    assert!(preview["archive_approx_tokens"].as_u64().unwrap() > 0);

    let flush = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/distill/flush")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(flush["queued_sessions"].as_u64().unwrap() >= 1);
}
