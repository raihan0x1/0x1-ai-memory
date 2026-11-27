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
use ox1_memory::updater::UpdateManager;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
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
    let updater = Arc::new(UpdateManager::new(
        db.clone(),
        PathBuf::from("/usr/local/bin/0x1-memory"),
        tmp.to_path_buf(),
        tmp.join("daemon.log"),
        10101,
        shutdown_tx.clone(),
        events_tx.clone(),
    ));
    AppState {
        db,
        embedder,
        llm,
        distill,
        connectors,
        autostart,
        updater,
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
async fn daily_caps_can_be_disabled_independently() {
    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    state
        .db
        .record_usage(
            "openrouter",
            "test-model",
            "distill",
            900,
            200,
            0.75,
            None,
            "ok",
            None,
        )
        .unwrap();
    let app = build_router(state);

    for (token_cap, saved_token_cap, usd_cap, exhausted) in [
        (0, 0, 0.50, true),
        (1, 1_000, 0.0, true),
        (0, 0, 0.0, false),
        (0, 0, 0.00001, true),
    ] {
        let save = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/settings")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "daily_token_cap": token_cap, "daily_cost_cap_usd": usd_cap })
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(save.status(), StatusCode::OK);

        let summary = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/summary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(summary.status(), StatusCode::OK);
        let summary = body_json(summary).await;
        assert_eq!(summary["daily_token_cap"], saved_token_cap);
        assert_eq!(summary["daily_cost_cap_usd"], usd_cap);
        assert_eq!(summary["budget_exhausted"], exhausted);
    }
}

#[tokio::test]
async fn end_to_end_cross_client_mcp_hooks_ui_and_budget_guard() {
    let tmp = tempdir().unwrap();
    let repo_dir = tmp.path().join("demo-repo");
    tokio::fs::create_dir_all(&repo_dir).await.unwrap();
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
    assert!(html.contains("<title>AI Memory</title>"));
    assert!(html.contains("/assets/"));
    assert!(html.contains("href=\"/favicon.ico\""));

    let favicon = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/favicon.ico")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(favicon.status(), StatusCode::OK);
    assert!(
        favicon.headers()[axum::http::header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("image/")
    );
    let icon = favicon.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(icon.as_ref(), include_bytes!("../ui/public/favicon.ico"));

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

    // A ledger read failure defers distillation and preserves the raw events.
    state.db.set_project_privacy(&proj_id, "full").unwrap();
    settings.provider = "openai_compatible".into();
    settings.base_url = "http://127.0.0.1:0/v1".into();
    state.db.save_settings(&settings).unwrap();
    state
        .db
        .insert_raw_event(&sess.id, &proj_id, "claude-code", "prompt", None, "{}")
        .unwrap();
    rusqlite::Connection::open(tmp.path().join("memory.db"))
        .unwrap()
        .execute_batch("DROP TABLE usage_ledger")
        .unwrap();
    distill_session_once(
        &state.db,
        &state.embedder,
        &state.llm,
        &state.events_tx,
        &sess.id,
    )
    .await;
    assert_eq!(
        state
            .db
            .pending_events_for_session(&sess.id, 10)
            .unwrap()
            .len(),
        1
    );
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
                let content = if payload["model"] == "invalid-output" {
                    json!({"ok": true})
                } else {
                    json!({"memories": [{
                        "scope": "project",
                        "type": "bugfix",
                        "title": "Validate inputs",
                        "summary": "Validate inputs before writing",
                        "content": "The test application now validates inputs before writing a file."
                    }]})
                };
                captured_srv.lock().unwrap().push((auth, payload));
                axum::Json(json!({
                    "choices": [{ "message": { "content": content.to_string() } }],
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
    assert_eq!(draft_json["estimated_cost_usd"], 0.00001);
    assert_eq!(draft_json["status"], "ok");
    assert_eq!(
        state.db.recent_usage(1).unwrap()[0].estimated_cost_usd,
        0.00001
    );

    // Saved DB settings remain untouched (still Zero-LLM with empty key)
    let saved_after = state.db.get_settings().unwrap();
    assert_eq!(saved_after.provider, "none");
    assert_eq!(saved_after.api_key, "");

    // A model-only draft keeps the key for the saved endpoint.
    let mut stored = state.db.get_settings().unwrap();
    stored.provider = "openrouter".into();
    stored.base_url = format!("http://{mock_addr}/v1");
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
                        "model": "invalid-output",
                        "base_url": format!("http://{mock_addr}/v1")
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stored_key_resp.status(), StatusCode::OK);
    let invalid = body_json(stored_key_resp).await;
    assert_eq!(invalid["ok"], false);
    assert_eq!(invalid["status"], "error");
    assert_eq!(invalid["input_tokens"], 12);
    assert_eq!(invalid["output_tokens"], 4);
    assert_eq!(invalid["estimated_cost_usd"], 0.00001);
    let ledger = &state.db.recent_usage(1).unwrap()[0];
    assert_eq!(ledger.status, "error");
    assert_eq!(ledger.estimated_cost_usd, 0.00001);

    stored.daily_cost_cap_usd = 0.00002;
    state.db.save_settings(&stored).unwrap();
    let blocked_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/engine/test")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"daily_cost_cap_usd":0}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(blocked_resp.status(), StatusCode::OK);
    let blocked = body_json(blocked_resp).await;
    assert_eq!(blocked["ok"], false);
    assert_eq!(blocked["status"], "budget_blocked");
    assert_eq!(
        state.db.recent_usage(1).unwrap()[0].status,
        "budget_blocked"
    );
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
    tokio::fs::create_dir_all(&repo_dir).await.unwrap();
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

#[tokio::test]
async fn stale_daemon_health_triggers_upgrade_and_ui_cache_headers() {
    use axum::{Json, Router, routing::get, routing::post};

    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state);

    let root_resp = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(root_resp.status(), StatusCode::OK);
    assert_eq!(
        root_resp
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-cache")
    );

    let missing_asset = app
        .oneshot(
            Request::builder()
                .uri("/assets/does-not-exist.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_asset.status(), StatusCode::NOT_FOUND);

    // Mock a stale v0.2.3 daemon exposing /health and /api/daemon/stop on an ephemeral port
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let mock_app = Router::new()
        .route(
            "/health",
            get(|| async {
                Json(json!({
                    "ok": true,
                    "service": "0x1-ai-memory",
                    "version": "0.2.3"
                }))
            }),
        )
        .route(
            "/api/daemon/stop",
            post(move || {
                let tx = stop_tx.clone();
                async move {
                    let _ = tx.send(true);
                    Json(json!({ "stopping": true }))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, mock_app)
            .with_graceful_shutdown(async move {
                while !*stop_rx.borrow() {
                    if stop_rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
    });

    let health = ox1_memory::daemon::fetch_daemon_health(port)
        .await
        .expect("mock stale daemon should answer /health");
    assert_eq!(health.version.as_deref(), Some("0.2.3"));
    assert!(ox1_memory::daemon::daemon_needs_upgrade(
        health.version.as_deref(),
        env!("CARGO_PKG_VERSION")
    ));

    let paths = ox1_memory::config::AppPaths {
        data_dir: tmp.path().to_path_buf(),
        db_path: tmp.path().join("memory.db"),
        log_path: tmp.path().join("daemon.log"),
        pid_path: tmp.path().join("daemon.pid"),
        backups_dir: tmp.path().join("backups"),
        models_dir: tmp.path().join("models"),
    };
    assert!(ox1_memory::daemon::stop_daemon(&paths, port).await.unwrap());
    assert!(
        ox1_memory::daemon::fetch_daemon_health(port)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn search_filters_before_selecting_candidates() {
    use ox1_memory::config::EMBEDDING_DIM;
    use ox1_memory::models::{NewMemoryInput, SearchFilter};
    use ox1_memory::project::ProjectIdentity;

    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    for id in ["proj_wanted", "proj_distractor"] {
        state
            .db
            .upsert_project(&ProjectIdentity {
                id: id.into(),
                name: id.into(),
                remote_url: None,
                root_path: format!("/tmp/{id}"),
                branch: None,
            })
            .unwrap();
    }

    let create = |project_id: Option<&str>, title: &str, tag: Option<&str>, embedding| {
        state
            .db
            .create_memory(
                NewMemoryInput {
                    scope: None,
                    project_id: project_id.map(str::to_owned),
                    session_id: None,
                    source_client: None,
                    memory_type: "decision".into(),
                    title: title.into(),
                    summary: String::new(),
                    content: String::new(),
                    files: None,
                    tags: tag.map(|tag| vec![tag.to_owned()]),
                    status: None,
                    supersedes_id: None,
                },
                embedding,
            )
            .unwrap()
            .id
    };

    let mut query_embedding = [0.0_f32; EMBEDDING_DIM];
    query_embedding[0] = 1.0;
    let mut relevant_embedding = [0.0_f32; EMBEDDING_DIM];
    relevant_embedding[0] = 0.8;
    relevant_embedding[1] = 0.6;
    // These matches rank higher than the wanted memory in both FTS and KNN.
    for _ in 0..60 {
        create(
            Some("proj_distractor"),
            "candidatetoken",
            None,
            Some(query_embedding.as_slice()),
        );
    }
    let relevant = create(
        Some("proj_wanted"),
        "A reference note about candidatetoken",
        None,
        Some(relevant_embedding.as_slice()),
    );
    let global = create(None, "A global note about candidatetoken", None, None);
    let literal = create(
        Some("proj_wanted"),
        "precandidatetoken",
        Some("literal"),
        None,
    );

    let db = state.db.clone();
    let app = build_router(state);
    for (suffix, expected) in [
        ("", vec![&relevant, &global]),
        ("&tag=literal", vec![&literal]),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/memories?project_id=proj_wanted&query=candidatetoken{suffix}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["total"], expected.len(), "{suffix}");
        let hits = body["memories"].as_array().unwrap();
        assert_eq!(hits.len(), expected.len(), "{suffix}");
        for id in expected {
            assert!(hits.iter().any(|hit| hit["id"] == id.as_str()), "{suffix}");
        }
    }

    let (hits, total) = db
        .search_memories_paged(
            &SearchFilter {
                query: Some("unmatchedsemanticintent".into()),
                project_id: Some("proj_wanted".into()),
                ..SearchFilter::default()
            },
            Some(&query_embedding),
        )
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(hits[0].id, relevant);
}

#[tokio::test]
async fn graph_filters_before_limiting_and_reports_total_matches() {
    use ox1_memory::models::NewMemoryInput;

    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    let create = |project_id: &str, title: &str| {
        state
            .db
            .create_memory(
                NewMemoryInput {
                    scope: None,
                    project_id: Some(project_id.into()),
                    session_id: None,
                    source_client: None,
                    memory_type: "decision".into(),
                    title: title.into(),
                    summary: String::new(),
                    content: String::new(),
                    files: None,
                    tags: None,
                    status: None,
                    supersedes_id: None,
                },
                None,
            )
            .unwrap()
            .id
    };
    let first = create("proj_graph", "graphneedle recalled");
    let second = create("proj_graph", "graphneedle fresh");
    create("proj_graph", "other topic");
    for _ in 0..501 {
        create("proj_other", "graphneedle distractor");
    }
    state
        .db
        .record_memory_recalls(std::slice::from_ref(&first), "codex")
        .unwrap();

    let app = build_router(state);
    for (suffix, expected_total, expected_id) in [
        ("&sort=most_recalled", 3, Some(&first)),
        ("&query=graphneedle&sort=most_recalled", 2, Some(&first)),
        ("&query=graphneedle&sort=never_recalled", 1, Some(&second)),
        ("&query=absentterm", 0, None),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/graph?project_id=proj_graph&limit=1{suffix}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let graph = body_json(response).await;
        assert_eq!(graph["total"], expected_total, "{suffix}");
        let nodes = graph["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), usize::from(expected_id.is_some()), "{suffix}");
        if let Some(id) = expected_id {
            assert_eq!(nodes[0]["id"], id.as_str(), "{suffix}");
        }
    }
}

#[tokio::test]
async fn memory_browse_pages_count_only_matching_rows() {
    use ox1_memory::models::NewMemoryInput;

    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    let create = |project_id: Option<&str>, status: &str, tag: &str, file: &str| {
        state
            .db
            .create_memory(
                NewMemoryInput {
                    scope: None,
                    project_id: project_id.map(str::to_owned),
                    session_id: None,
                    source_client: Some("codex".into()),
                    memory_type: "decision".into(),
                    title: status.into(),
                    summary: String::new(),
                    content: String::new(),
                    files: Some(vec![file.into()]),
                    tags: Some(vec![tag.into()]),
                    status: Some(status.into()),
                    supersedes_id: None,
                },
                None,
            )
            .unwrap()
            .id
    };
    let scoped = create(Some("proj_browse"), "active", "rust", "src/%tool.rs");
    let global = create(None, "pinned", "rust", "src/%tool.rs");
    create(Some("proj_browse"), "archived", "rust", "src/%tool.rs");
    create(Some("proj_browse"), "active", "rust", "src/buildtool.rs");
    create(Some("proj_browse"), "active", "rusty", "src/%tool.rs");
    create(Some("other_proj"), "active", "rust", "src/%tool.rs");
    state
        .db
        .record_memory_recalls(std::slice::from_ref(&scoped), "codex")
        .unwrap();

    let app = build_router(state);
    let oversized = format!("&offset={}", usize::MAX);
    for (suffix, total, expected) in [
        ("", 2, Some(&global)),
        ("&sort=most_recalled", 2, Some(&scoped)),
        ("&offset=1&sort=most_recalled", 2, Some(&global)),
        ("&sort=never_recalled", 1, Some(&global)),
        ("&scope=project", 1, Some(&scoped)),
        ("&include_inactive=true", 3, Some(&global)),
        ("&memory_type=architecture", 0, None),
        (oversized.as_str(), 2, None),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/memories?project_id=proj_browse&tag=%20RUST%20&file=%25TOOL&limit=1{suffix}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{suffix}");
        let page = body_json(response).await;
        assert_eq!(page["total"], total, "{suffix}");
        assert_eq!(page["count"], usize::from(expected.is_some()), "{suffix}");
        let first = page["memories"][0]["id"].as_str();
        assert_eq!(first, expected.map(String::as_str), "{suffix}");
    }
}

#[tokio::test]
async fn pagination_across_memories_sessions_raw_events_and_usage() {
    use ox1_memory::models::NewMemoryInput;
    use ox1_memory::project::ProjectIdentity;
    use std::collections::HashSet;

    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());

    let p1 = state
        .db
        .upsert_project(&ProjectIdentity {
            id: "proj_pag_1".into(),
            name: "pag-one".into(),
            remote_url: None,
            root_path: "/tmp/pag-one".into(),
            branch: Some("main".into()),
        })
        .unwrap();
    let p2 = state
        .db
        .upsert_project(&ProjectIdentity {
            id: "proj_pag_2".into(),
            name: "pag-two".into(),
            remote_url: None,
            root_path: "/tmp/pag-two".into(),
            branch: Some("main".into()),
        })
        .unwrap();

    // Seed 25 memories across 2 projects & clients (with searchable keyword "pagtoken")
    for i in 0..25 {
        let proj = if i % 2 == 0 { &p1.id } else { &p2.id };
        let client = if i % 3 == 0 { "codex" } else { "claude-code" };
        state
            .db
            .create_memory(
                NewMemoryInput {
                    scope: Some("project".into()),
                    project_id: Some(proj.clone()),
                    session_id: None,
                    source_client: Some(client.into()),
                    memory_type: "decision".into(),
                    title: format!("Paginated memory #{i:02} pagtoken"),
                    summary: format!("Summary for memory {i} pagtoken"),
                    content: format!("Detailed content {i} pagtoken"),
                    files: None,
                    tags: None,
                    status: Some("active".into()),
                    supersedes_id: None,
                },
                None,
            )
            .unwrap();
    }

    // Seed 10 sessions
    let mut sess_ids = Vec::new();
    for i in 0..10 {
        let proj = if i % 2 == 0 { &p1.id } else { &p2.id };
        let s = state
            .db
            .upsert_session(proj, "claude-code", &format!("ext-sess-{i:02}"), None)
            .unwrap();
        sess_ids.push(s.id);
    }

    // Seed 25 raw events (mixed prompt, tool_use, turn_stop)
    for i in 0..25 {
        let (etype, tool) = match i % 3 {
            0 => ("prompt", None),
            1 => ("tool_use", Some("Bash")),
            _ => ("turn_stop", None),
        };
        let sid = &sess_ids[i % sess_ids.len()];
        let proj = if (i % sess_ids.len()) % 2 == 0 {
            &p1.id
        } else {
            &p2.id
        };
        state
            .db
            .insert_raw_event(sid, proj, "claude-code", etype, tool, "{}")
            .unwrap();
    }

    // Seed 55 usage rows (> old 50 cap)
    for i in 0..55 {
        state
            .db
            .record_usage(
                "openrouter",
                "test-model",
                &format!("op-{i}"),
                100,
                50,
                0.0001,
                None,
                "ok",
                None,
            )
            .unwrap();
    }

    let app = build_router(state);

    // 1. Memories pagination (unfiltered + offset)
    let m_p1 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/memories?limit=10&offset=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(m_p1["total"], 25);
    assert_eq!(m_p1["count"], 10);
    assert_eq!(m_p1["limit"], 10);
    assert_eq!(m_p1["offset"], 0);
    let p1_ids: HashSet<String> = m_p1["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(p1_ids.len(), 10);

    let m_p3 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/memories?limit=10&offset=20")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(m_p3["total"], 25);
    assert_eq!(m_p3["count"], 5);
    assert_eq!(m_p3["limit"], 10);
    assert_eq!(m_p3["offset"], 20);
    let p3_ids: HashSet<String> = m_p3["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(p3_ids.len(), 5);
    assert!(p1_ids.is_disjoint(&p3_ids));

    // Search query pagination
    let mq_p1 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/memories?query=pagtoken&limit=5&offset=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let mq_p2 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/memories?query=pagtoken&limit=5&offset=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(mq_p1["total"], 25);
    assert_eq!(mq_p1["memories"].as_array().unwrap().len(), 5);
    assert_eq!(mq_p2["total"], 25);
    assert_eq!(mq_p2["memories"].as_array().unwrap().len(), 5);
    let mq1_ids: HashSet<String> = mq_p1["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    let mq2_ids: HashSet<String> = mq_p2["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert!(mq1_ids.is_disjoint(&mq2_ids));

    // 2. Sessions pagination
    let s_p1 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/sessions?limit=6&offset=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let s_p2 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/sessions?limit=6&offset=6")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(s_p1["total"], 10);
    assert_eq!(s_p1["sessions"].as_array().unwrap().len(), 6);
    assert_eq!(s_p2["total"], 10);
    assert_eq!(s_p2["sessions"].as_array().unwrap().len(), 4);
    let s1_ids: HashSet<String> = s_p1["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    let s2_ids: HashSet<String> = s_p2["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    assert!(s1_ids.is_disjoint(&s2_ids));

    // 3. Raw events keyset pagination + kind=prompts filter (indices 0,3,6,9,12,15,18,21,24 => 9 prompts)
    let r_p1 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/raw-events?limit=5&kind=prompts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let cursor = r_p1["next_cursor"].as_str().unwrap();
    let r_p2 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/raw-events?limit=5&kind=prompts&before={cursor}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(r_p1["total"], 9);
    assert_eq!(r_p1["events"].as_array().unwrap().len(), 5);
    assert_eq!(r_p2["total"], 9);
    assert_eq!(r_p2["events"].as_array().unwrap().len(), 4);
    assert!(r_p2["next_cursor"].is_null());
    let r1_ids: HashSet<String> = r_p1["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    let r2_ids: HashSet<String> = r_p2["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    assert!(r1_ids.is_disjoint(&r2_ids));

    // 4. Usage ledger pagination (>50 rows)
    let u_p3 = body_json(
        app.oneshot(
            Request::builder()
                .uri("/api/usage?limit=20&offset=40")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(u_p3["total"], 55);
    assert_eq!(u_p3["limit"], 20);
    assert_eq!(u_p3["offset"], 40);
    assert_eq!(u_p3["usage"].as_array().unwrap().len(), 15);
}

#[tokio::test]
async fn updater_checks_etag_dismisses_seeds_whats_new_and_verifies_swap() {
    use axum::Router;
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use ox1_memory::updater::{ReleaseInfo, platform_asset_name, sha256_hex};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let tmp = tempdir().unwrap();
    let bin_dir = tmp.path().join("bin");
    tokio::fs::create_dir_all(&bin_dir).await.unwrap();
    let exe_path = bin_dir.join(if cfg!(windows) {
        "0x1-memory.exe"
    } else {
        "0x1-memory"
    });
    tokio::fs::write(&exe_path, b"old-binary-bytes")
        .await
        .unwrap();

    // Prepare mock archive + SHA256SUMS.txt (real tar.gz on Unix; bogus bytes for cross-platform failure check)
    let asset = platform_asset_name().unwrap_or("0x1-memory-darwin-arm64.tar.gz");
    let archive_bytes: Vec<u8> = {
        #[cfg(unix)]
        {
            let stage_dir = tmp.path().join("pkg");
            tokio::fs::create_dir_all(&stage_dir).await.unwrap();
            let mock_bin = stage_dir.join("0x1-memory");
            tokio::fs::write(&mock_bin, "#!/bin/sh\necho \"0x1-memory 9.9.9\"\n")
                .await
                .unwrap();
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&mock_bin, std::fs::Permissions::from_mode(0o700))
                .await
                .unwrap();
            let archive_file = tmp.path().join("pkg.tar.gz");
            let status = tokio::process::Command::new("/usr/bin/tar")
                .args([
                    "-czf",
                    archive_file.to_str().unwrap(),
                    "-C",
                    stage_dir.to_str().unwrap(),
                    "0x1-memory",
                ])
                .status()
                .await
                .unwrap();
            assert!(status.success());
            tokio::fs::read(archive_file).await.unwrap()
        }
        #[cfg(not(unix))]
        {
            b"not-a-real-archive".to_vec()
        }
    };
    let good_sha = sha256_hex(&archive_bytes);
    let good_sums = format!("{good_sha}  {asset}\n");
    let bad_sums = format!("{}  {asset}\n", "0".repeat(64));

    let latest_calls = Arc::new(AtomicUsize::new(0));
    let latest_calls_srv = latest_calls.clone();
    let sums_payload = Arc::new(tokio::sync::RwLock::new(good_sums.clone()));
    let sums_srv = sums_payload.clone();
    let archive_srv = archive_bytes.clone();

    let mock_gh = Router::new()
        .route(
            "/repos/raihan0x1/0x1-ai-memory/releases/latest",
            get(move |headers: HeaderMap| {
                let calls = latest_calls_srv.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    if headers
                        .get("if-none-match")
                        .and_then(|v| v.to_str().ok())
                        == Some("\"etag-999\"")
                    {
                        return StatusCode::NOT_MODIFIED.into_response();
                    }
                    (
                        [(axum::http::header::ETAG, "\"etag-999\"")],
                        axum::Json(json!({
                            "tag_name": "v9.9.9",
                            "name": "v9.9.9",
                            "body": "## What's Changed\n- Added pagination everywhere\n- Added 1-click Daemon update",
                            "html_url": "https://github.com/raihan0x1/0x1-ai-memory/releases/tag/v9.9.9",
                            "published_at": "2026-11-25T10:00:00Z",
                            "draft": false,
                            "prerelease": false
                        })),
                    )
                        .into_response()
                }
            }),
        )
        .route(
            "/repos/raihan0x1/0x1-ai-memory/releases/tags/v0.3.1",
            get(|| async {
                axum::Json(json!({
                    "tag_name": "v0.3.1",
                    "name": "v0.3.1",
                    "body": "- Current release highlights",
                    "html_url": "https://github.com/raihan0x1/0x1-ai-memory/releases/tag/v0.3.1",
                    "draft": false,
                    "prerelease": false
                }))
            }),
        )
        .route(
            "/releases/download/v9.9.9/SHA256SUMS.txt",
            get(move || {
                let s = sums_srv.clone();
                async move { s.read().await.clone() }
            }),
        )
        .route(
            &format!("/releases/download/v9.9.9/{asset}"),
            get(move || {
                let bytes = archive_srv.clone();
                async move { bytes }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, mock_gh).await;
    });

    let base_api = format!("http://127.0.0.1:{mock_port}");
    let base_rel = format!("http://127.0.0.1:{mock_port}/releases/download");

    // 1. Fresh DB startup skips "What's new"
    let mut state = test_state(tmp.path());
    let updater = Arc::new(UpdateManager::with_urls_and_version(
        state.db.clone(),
        exe_path.clone(),
        tmp.path().to_path_buf(),
        tmp.join_log(),
        10101,
        state.shutdown_tx.clone(),
        state.events_tx.clone(),
        "0.3.1".into(),
        base_api.clone(),
        base_rel.clone(),
    ));
    updater.seed_version_state_on_startup(false).unwrap();
    let st_fresh = updater.status().await;
    assert!(!st_fresh.whats_new.should_show);

    // Simulate upgraded DB (previously saw 0.3.0)
    state.db.set_meta("last_seen_version", "0.3.0").unwrap();
    updater.seed_version_state_on_startup(false).unwrap();
    state.updater = updater.clone();
    let app = build_router(state.clone());

    let st_up = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/updates/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st_up["whats_new"]["should_show"], true);
    assert_eq!(st_up["whats_new"]["previous_version"], "0.3.0");

    // Fetch current version release notes & ack What's New
    let fetched = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/updates/fetch-whats-new")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(
        fetched["whats_new"]["body"]
            .as_str()
            .unwrap()
            .contains("Current release highlights")
    );

    let acked = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/updates/ack-whats-new")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(acked["whats_new"]["should_show"], false);

    // 2. Check for updates -> 200 OK caches ETag; second forced check gets 304 Not Modified
    let check1 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/updates/check")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(check1["update_available"], true);
    assert_eq!(check1["latest_release"]["version"], "9.9.9");

    let check2 = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/updates/check")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(latest_calls.load(Ordering::SeqCst), 2);
    assert_eq!(check2["update_available"], true);
    assert_eq!(check2["latest_release"]["version"], "9.9.9");

    // 3. Dismiss version persists
    let dismissed = body_json(
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/updates/dismiss")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "version": "9.9.9" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(dismissed["dismissed_version"], "9.9.9");

    // 4. Fail-closed SHA-256 mismatch leaves active binary untouched
    let target = ReleaseInfo {
        tag_name: "v9.9.9".into(),
        version: "9.9.9".into(),
        name: "v9.9.9".into(),
        body: "Release notes for 9.9.9".into(),
        html_url: "https://github.com/raihan0x1/0x1-ai-memory/releases/tag/v9.9.9".into(),
        published_at: None,
    };
    *sums_payload.write().await = bad_sums;
    let fail_res = updater.stage_verify_and_swap(&target, asset).await;
    assert!(fail_res.is_err());
    assert_eq!(
        tokio::fs::read(&exe_path).await.unwrap(),
        b"old-binary-bytes"
    );

    // 5. Valid checksum + --version smoke check swaps binary, updates .version stamp, and caches notes
    #[cfg(unix)]
    {
        *sums_payload.write().await = good_sums;
        updater
            .stage_verify_and_swap(&target, asset)
            .await
            .expect("valid archive & checksum should swap binary");
        let swapped = tokio::fs::read_to_string(&exe_path).await.unwrap();
        assert!(swapped.contains("0x1-memory 9.9.9"));
        use std::os::unix::fs::PermissionsExt;
        let swapped_mode = tokio::fs::metadata(&exe_path)
            .await
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(swapped_mode, 0o700);
        let stamp = tokio::fs::read_to_string(bin_dir.join(".version"))
            .await
            .unwrap();
        assert_eq!(stamp.trim(), "9.9.9");
        let cached_notes = state
            .db
            .get_meta("whats_new_cache_json")
            .unwrap()
            .expect("release notes should be pre-cached before restart");
        assert!(cached_notes.contains("Release notes for 9.9.9"));
    }
}

trait TmpPathHelper {
    fn join_log(&self) -> PathBuf;
}

impl TmpPathHelper for tempfile::TempDir {
    fn join_log(&self) -> PathBuf {
        self.path().join("daemon.log")
    }
}

#[tokio::test]
async fn distillation_sends_every_event_once_and_guards_supersede() {
    use axum::{Router, extract::Request as AxumRequest, routing::post};
    use std::sync::{Arc, Mutex};

    // Mock LLM records every user prompt and supersedes whichever memory id the test sets.
    let prompts = Arc::new(Mutex::new(Vec::<String>::new()));
    let supersede = Arc::new(Mutex::new(None::<String>));
    let (prompts_srv, supersede_srv) = (prompts.clone(), supersede.clone());
    let mock_router = Router::new().route(
        "/v1/chat/completions",
        post(move |req: AxumRequest| {
            let (prompts_srv, supersede_srv) = (prompts_srv.clone(), supersede_srv.clone());
            async move {
                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                let payload: Value = serde_json::from_slice(&bytes).unwrap_or_default();
                let user = payload["messages"][1]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                prompts_srv.lock().unwrap().push(user);
                let content = json!({"memories": [{
                    "scope": "project",
                    "type": "decision",
                    "title": "Batch decision",
                    "summary": "A decision from this batch",
                    "content": "Details of the batch decision.",
                    "supersedes_id": supersede_srv.lock().unwrap().clone()
                }]});
                axum::Json(json!({
                    "choices": [{ "message": { "content": content.to_string() } }],
                    "usage": { "prompt_tokens": 12, "completion_tokens": 4, "cost": 0.0 }
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
    let repo_a = tmp.path().join("repo-a");
    let repo_b = tmp.path().join("repo-b");
    tokio::fs::create_dir_all(&repo_a).await.unwrap();
    tokio::fs::create_dir_all(&repo_b).await.unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());
    let mut settings = state.db.get_settings().unwrap();
    settings.provider = "openai_compatible".into();
    settings.base_url = format!("http://{mock_addr}/v1");
    settings.model = "mock-model".into();
    state.db.save_settings(&settings).unwrap();

    let post_tool = |marker: String| {
        let app = app.clone();
        let cwd = repo_a.to_string_lossy().to_string();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/hooks/claude?event=PostToolUse")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({
                                "cwd": cwd,
                                "session_id": "batch-sess",
                                "tool_name": "Bash",
                                "tool_input": { "command": format!("echo {marker}") },
                                "tool_response": "x".repeat(1_500)
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    };
    let pending = || {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri("/api/sessions?limit=10")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            body_json(resp).await["sessions"][0]["pending_events"]
                .as_i64()
                .unwrap()
        }
    };

    // 1. More events than fit one prompt: every event reaches the model exactly once.
    let markers: Vec<String> = (0..40).map(|i| format!("EVT-{i:02}-END")).collect();
    for m in &markers {
        post_tool(m.clone()).await;
    }
    let project_a = ox1_memory::project::resolve_project(&repo_a).id;
    let sess = state
        .db
        .upsert_session(&project_a, "claude-code", "batch-sess", None)
        .unwrap();
    for _ in 0..40 {
        if pending().await == 0 {
            break;
        }
        distill_session_once(
            &state.db,
            &state.embedder,
            &state.llm,
            &state.events_tx,
            &sess.id,
        )
        .await;
    }
    assert_eq!(pending().await, 0);
    let sent = prompts.lock().unwrap().clone();
    assert!(
        sent.len() > 1,
        "40 large events fit {} prompt(s)",
        sent.len()
    );
    for m in &markers {
        let hits = sent.iter().filter(|p| p.contains(m.as_str())).count();
        assert_eq!(hits, 1, "{m} reached the model {hits} times");
    }

    // 2. Supersede only replaces an active memory in the same project and scope.
    let create = |body: Value| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/memories")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            body_json(resp).await["memory"]["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let status_of = |id: String| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/memories/{id}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            body_json(resp).await["memory"]["status"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let project_b = ox1_memory::project::resolve_project(&repo_b).id;
    let memory = |project: &str, status: &str, title: &str| {
        json!({
            "scope": "project", "project_id": project, "status": status,
            "memory_type": "decision", "title": title,
            "summary": "s", "content": "c"
        })
    };
    let pinned = create(memory(&project_a, "pinned", "Pinned rule")).await;
    let other_project = create(memory(&project_b, "active", "Other project rule")).await;
    let replaceable = create(memory(&project_a, "active", "Old rule")).await;

    for target in [&pinned, &other_project, &replaceable] {
        *supersede.lock().unwrap() = Some(target.clone());
        post_tool(format!("supersede {target}")).await;
        distill_session_once(
            &state.db,
            &state.embedder,
            &state.llm,
            &state.events_tx,
            &sess.id,
        )
        .await;
    }
    assert_eq!(status_of(pinned).await, "pinned");
    assert_eq!(status_of(other_project).await, "active");
    assert_eq!(status_of(replaceable).await, "superseded");

    mock_task.abort();
}

#[tokio::test]
async fn legacy_client_names_are_normalized_and_t3_rows_removed_on_open() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    tokio::fs::create_dir_all(&repo).await.unwrap();
    let project = ox1_memory::project::resolve_project(&repo).id;
    {
        // Seed data the old binary could write: alias Client names, a T3 Code connection,
        // and two sessions that collide once their names are normalized.
        let state = test_state(tmp.path());
        let app = build_router(state.clone());
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/hooks/claude?event=UserPromptSubmit")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"cwd": repo, "session_id": "s-alias", "prompt": "hello"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let memory = json!({
            "scope": "project", "project_id": project, "memory_type": "decision",
            "title": "Legacy memory", "summary": "s", "content": "c"
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/memories")
                    .header("content-type", "application/json")
                    .body(Body::from(memory.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        state
            .db
            .upsert_session(&project, "codex", "dup", None)
            .unwrap();
        state
            .db
            .upsert_session(&project, "claude-code", "dup", None)
            .unwrap();
        let raw = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
        raw.execute_batch(
            "UPDATE memories SET source_client = 'go-http-client/1.1';
             UPDATE raw_events SET source_client = 'antigravity-client';
             UPDATE sessions SET source_client = 'antigravity-client' WHERE external_session_id = 's-alias';
             UPDATE sessions SET source_client = 'codex-mcp-client'
               WHERE external_session_id = 'dup' AND source_client = 'claude-code';
             INSERT INTO client_connections(client_id, display_name, detected, connected, updated_at) VALUES
               ('t3-code', 'T3 Code', 1, 1, '2026-01-01T00:00:00Z'),
               ('codex-mcp-client', 'codex-mcp-client', 1, 1, '2026-01-01T00:00:00Z'),
               ('go-http-client/1.1', 'go-http-client/1.1', 1, 1, '2026-01-01T00:00:00Z'),
               ('codex', 'Codex CLI & App', 1, 1, '2026-01-01T00:00:00Z');",
        )
        .unwrap();
    }

    // Reopening runs the migration; the Daemon must start and serve canonical names.
    let state = test_state(tmp.path());
    let app = build_router(state.clone());
    let get = |uri: &'static str| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "{uri}");
            body_json(resp).await
        }
    };
    let memories = get("/api/memories?limit=10").await;
    assert_eq!(memories["memories"][0]["source_client"], "mcp");
    let events = get("/api/raw-events?limit=10").await;
    assert_eq!(events["events"][0]["source_client"], "antigravity");
    let sessions = get("/api/sessions?limit=10").await;
    let mut names: Vec<String> = sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["source_client"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    // The colliding alias session is kept as-is rather than merged or dropped.
    assert_eq!(names, ["antigravity", "codex", "codex-mcp-client"]);

    let connections = get("/api/connections").await;
    let shown: Vec<&str> = connections["connections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["client_id"].as_str().unwrap())
        .collect();
    assert!(!shown.contains(&"t3-code"), "{shown:?}");
    let raw = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
    let mut stmt = raw
        .prepare("SELECT client_id FROM client_connections ORDER BY client_id")
        .unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows, ["antigravity", "claude-code", "codex"]);

    // An MCP caller identified only by a generic User-Agent is attributed to `mcp`
    // and does not appear as a Client connection.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("user-agent", "Go-http-client/1.1")
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": { "name": "memory_save", "arguments": {
                            "title": "From a generic MCP caller", "summary": "s",
                            "content": "c", "type": "decision", "scope": "project",
                            "project": repo.to_string_lossy()
                        }}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let memories = get("/api/memories?limit=10").await;
    assert!(
        memories["memories"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["source_client"] == "mcp")
    );
    let count: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM client_connections WHERE client_id NOT IN ('claude-code','codex','antigravity')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn briefing_recall_and_mcp_search_carry_generated_at_and_memory_ages() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    tokio::fs::create_dir_all(&repo).await.unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());
    let project = ox1_memory::project::resolve_project(&repo).id;
    let call = |uri: &'static str, body: Value| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "{uri}");
            body_json(resp).await
        }
    };
    for (title, summary) in [
        (
            "Webhook retries use backoff",
            "Retries back off exponentially",
        ),
        (
            "Webhook signatures are verified",
            "Signatures use HMAC SHA256",
        ),
    ] {
        call(
            "/api/memories",
            json!({
                "scope": "project", "project_id": project, "memory_type": "decision",
                "title": title, "summary": summary, "content": "c"
            }),
        )
        .await;
    }
    // Age the two memories: one 14 minutes, one two days before now.
    let now = chrono::Utc::now();
    let raw = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
    for (title, age) in [
        ("Webhook retries use backoff", chrono::Duration::minutes(14)),
        ("Webhook signatures are verified", chrono::Duration::days(2)),
    ] {
        raw.execute(
            "UPDATE memories SET updated_at = ?1 WHERE title = ?2",
            rusqlite::params![(now - age).to_rfc3339(), title],
        )
        .unwrap();
    }
    let local = |age: chrono::Duration| (now - age).with_timezone(&chrono::Local);
    let recent_clock = local(chrono::Duration::minutes(14))
        .format("%H:%M")
        .to_string();
    let old_stamp = local(chrono::Duration::days(2))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    let check = |text: &str, reference: &str| {
        let first = text
            .lines()
            .find(|l| l.starts_with("Generated at "))
            .unwrap_or_else(|| panic!("no generated-at line in:\n{text}"));
        assert!(first.contains(&today), "{first}");
        let recent = text
            .lines()
            .find(|l| l.contains("Webhook retries use backoff"))
            .unwrap();
        assert!(
            recent.contains(&format!("{recent_clock}, 14m before {reference}")),
            "{recent}"
        );
        let old = text
            .lines()
            .find(|l| l.contains("Webhook signatures are verified"))
            .unwrap();
        assert!(
            old.contains(&format!("{old_stamp}, 2d 0h before {reference}")),
            "{old}"
        );
        assert!(!text.contains(" ago"), "{text}");
    };

    let briefing = call(
        "/v1/hooks/claude?event=SessionStart",
        json!({"cwd": repo, "session_id": "time-sess"}),
    )
    .await;
    check(
        briefing["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap(),
        "this briefing",
    );

    let recall = call(
        "/v1/hooks/claude?event=UserPromptSubmit",
        json!({"cwd": repo, "session_id": "time-sess", "prompt": "how do webhook retries and signatures work"}),
    )
    .await;
    check(
        recall["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap(),
        "this recall",
    );

    let search = call(
        "/mcp",
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "memory_search", "arguments": {
                "query": "webhook", "project": repo.to_string_lossy()
            }}
        }),
    )
    .await;
    let text = search["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    let mut lines = vec![format!(
        "Generated at {}",
        parsed["generated_at"].as_str().unwrap()
    )];
    for r in parsed["results"].as_array().unwrap() {
        lines.push(format!(
            "{} ({})",
            r["title"].as_str().unwrap(),
            r["updated"].as_str().unwrap()
        ));
    }
    check(&lines.join("\n"), "this search");
}

#[tokio::test]
async fn distill_results_are_stored_events_that_never_revive_a_session() {
    use axum::{Router, routing::post};

    let mock_router = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            let content = json!({"memories": [{
                "scope": "project", "type": "decision", "title": "Queue retries",
                "summary": "Retries go through the queue", "content": "Details."
            }]});
            axum::Json(json!({
                "choices": [{ "message": { "content": content.to_string() } }],
                "usage": { "prompt_tokens": 12, "completion_tokens": 4, "cost": 0.0 }
            }))
        }),
    );
    let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock_listener.local_addr().unwrap();
    let mock_task = tokio::spawn(async move {
        let _ = axum::serve(mock_listener, mock_router).await;
    });

    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    tokio::fs::create_dir_all(&repo).await.unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());
    let use_llm = |base_url: String| {
        let mut settings = state.db.get_settings().unwrap();
        settings.provider = "openai_compatible".into();
        settings.base_url = base_url;
        settings.model = "mock-model".into();
        state.db.save_settings(&settings).unwrap();
    };
    let hook = |event: &'static str, extra: Value| {
        let app = app.clone();
        let mut body = json!({"cwd": repo, "session_id": "c-sess"});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/v1/hooks/claude?event={event}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    };
    let get = |uri: &'static str| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            body_json(resp).await
        }
    };
    let tool = || json!({"tool_name": "Bash", "tool_input": {"command": "cargo test"}, "tool_response": "ok"});
    let sess = state
        .db
        .upsert_session(
            &ox1_memory::project::resolve_project(&repo).id,
            "claude-code",
            "c-sess",
            None,
        )
        .unwrap();
    let distill = || {
        distill_session_once(
            &state.db,
            &state.embedder,
            &state.llm,
            &state.events_tx,
            &sess.id,
        )
    };

    // One successful pass, then one failing pass (nothing listens on port 1).
    use_llm(format!("http://{mock_addr}/v1"));
    hook("PostToolUse", tool()).await;
    distill().await;
    use_llm("http://127.0.0.1:1/v1".into());
    hook("PostToolUse", tool()).await;
    let last_activity = get("/api/sessions?limit=5").await["sessions"][0]["last_turn_at"].clone();
    distill().await;

    let results = get("/api/raw-events?kind=distill").await;
    assert_eq!(results["total"], 2, "{results}");
    let rows = results["events"].as_array().unwrap();
    assert!(rows.iter().all(|r| r["event_type"] == "distill_result"));
    assert!(rows.iter().all(|r| r["distilled"] == true));
    assert!(
        rows[0]["preview"].as_str().unwrap().starts_with("Failed:"),
        "{}",
        rows[0]
    );
    assert_eq!(rows[1]["preview"], "1 memory from 1 event");

    // Daemon rows are not Client activity: no pending work and no revived session.
    let session = get("/api/sessions?limit=5").await["sessions"][0].clone();
    assert_eq!(session["pending_events"], 0);
    assert_eq!(session["last_turn_at"], last_activity);

    hook("Stop", json!({"last_assistant_message": "All tests pass."})).await;
    let turn_ends = get("/api/raw-events?kind=turn_end").await;
    assert_eq!(turn_ends["total"], 1, "{turn_ends}");
    assert_eq!(turn_ends["events"][0]["event_type"], "turn_stop");
    assert_eq!(turn_ends["events"][0]["preview"], "All tests pass.");

    mock_task.abort();
}

#[tokio::test]
async fn sessions_page_filter_by_client_and_derive_active_idle_ended() {
    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());

    // 12,000 sessions, session i last active i minutes ago; every 7th has ended.
    let clients = ["claude-code", "codex", "antigravity"];
    let now = chrono::Utc::now();
    {
        let mut raw = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
        let tx = raw.transaction().unwrap();
        for i in 1..=12_000i64 {
            let at = (now - chrono::Duration::minutes(i)).to_rfc3339();
            let ended = i % 7 == 0;
            tx.execute(
                "INSERT INTO sessions(id, project_id, source_client, external_session_id, started_at, ended_at, last_turn_at, status)
                 VALUES (?1, 'proj_big', ?2, ?1, ?3, ?4, ?3, ?5)",
                rusqlite::params![
                    format!("sess_{i:05}"),
                    clients[(i % 3) as usize],
                    at,
                    ended.then(|| at.clone()),
                    if ended { "ended" } else { "active" }
                ],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let get = |uri: String| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "{uri}");
            body_json(resp).await
        }
    };
    let ids = |page: &Value| -> Vec<String> {
        page["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_string())
            .collect()
    };

    let first = get("/api/sessions?limit=100".into()).await;
    assert_eq!(first["total"], 12_000);
    let expected: Vec<String> = (1..=100).map(|i| format!("sess_{i:05}")).collect();
    assert_eq!(ids(&first), expected, "newest activity first");
    let second = get("/api/sessions?limit=100&offset=100".into()).await;
    assert_eq!(ids(&second)[0], "sess_00101");
    let last = get("/api/sessions?limit=100&offset=11950".into()).await;
    assert_eq!(ids(&last).len(), 50);
    assert_eq!(ids(&last).last().unwrap(), "sess_12000");
    assert!(first["sessions"][0].get("total_events").is_none());

    // Status: ended wins; otherwise active within 30 minutes, else idle.
    let status = |page: &Value, id: &str| {
        page["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status(&first, "sess_00001"), "active");
    assert_eq!(status(&first, "sess_00007"), "ended");
    assert_eq!(status(&first, "sess_00029"), "active");
    assert_eq!(status(&first, "sess_00031"), "idle");
    // 1..=29 minutes ago minus the ended ones (7, 14, 21, 28).
    let summary = get("/api/summary".into()).await;
    assert_eq!(summary["active_sessions"], 25, "{summary}");

    let codex = get("/api/sessions?limit=100&client=codex".into()).await;
    assert_eq!(codex["total"], 4_000);
    assert!(
        codex["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["source_client"] == "codex")
    );
    assert_eq!(ids(&codex)[0], "sess_00001");
}

#[tokio::test]
async fn hook_records_the_host_that_launched_the_client() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    tokio::fs::create_dir_all(&repo).await.unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());
    for (sid, host) in [
        ("via-orca", "orca"),
        ("via-junk", "<script>"),
        ("plain", ""),
    ] {
        let mut body = json!({"cwd": repo, "session_id": sid, "prompt": "hello there"});
        if !host.is_empty() {
            body["ox1_host"] = host.into();
        }
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/hooks/claude?event=UserPromptSubmit")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let sessions = body_json(resp).await["sessions"].clone();
    let hosts: Vec<Value> = sessions
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["host"].clone())
        .collect();
    assert_eq!(hosts.len(), 3);
    assert_eq!(
        hosts.iter().filter(|h| **h == "orca").count(),
        1,
        "{sessions}"
    );
    // Unknown values are dropped rather than stored.
    assert_eq!(
        hosts.iter().filter(|h| h.is_null()).count(),
        2,
        "{sessions}"
    );
}

fn rescrub_backups(dir: &std::path::Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("memory.db.pre-rescrub-")
        })
        .map(|e| e.path())
        .collect()
}

#[tokio::test]
async fn one_time_rescrub_redacts_stored_secrets_keeps_search_in_sync_and_backs_up() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    tokio::fs::create_dir_all(&repo).await.unwrap();
    let project = ox1_memory::project::resolve_project(&repo).id;
    let send = |app: axum::Router, method: &'static str, uri: String, body: Option<Value>| async move {
        let mut req = Request::builder().method(method).uri(uri);
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        let resp = app
            .oneshot(
                req.body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        body_json(resp).await
    };
    let leaked =
        r#"curl -H "x-api-key: abcd1234efgh5678" https://admin:hunter2pass@api.example.com"#;
    {
        // A database written before the new rules existed: secrets stored in plain text.
        let state = test_state(tmp.path());
        let app = build_router(state.clone());
        send(
            app.clone(),
            "POST",
            "/v1/hooks/claude?event=PostToolUse".into(),
            Some(
                json!({"cwd": repo, "session_id": "old", "tool_name": "Bash",
                        "tool_input": {"command": "echo placeholder"}, "tool_response": "ok"}),
            ),
        )
        .await;
        for title in ["Deploy uses the admin API", "Model limits"] {
            send(
                app.clone(),
                "POST",
                "/api/memories".into(),
                Some(
                    json!({"scope": "project", "project_id": project, "memory_type": "decision",
                            "title": title, "summary": "s", "content": "placeholder"}),
                ),
            )
            .await;
        }
        let raw = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
        let payload = json!({"tool": "Bash", "input": {"command": leaked}, "output": "ok"});
        raw.execute(
            "UPDATE raw_events SET payload_json = ?1",
            rusqlite::params![payload.to_string()],
        )
        .unwrap();
        raw.execute(
            "UPDATE memories SET content = ?1 WHERE title LIKE 'Deploy%'",
            rusqlite::params![format!("Run {leaked} to deploy")],
        )
        .unwrap();
        raw.execute(
            "UPDATE memories SET content = 'Set MAX_TOKEN=4096 for the model' WHERE title = 'Model limits'",
            [],
        )
        .unwrap();
        raw.execute("DELETE FROM settings WHERE key LIKE '_meta:rescrub%'", [])
            .unwrap();
    }
    let backups = || rescrub_backups(tmp.path());
    assert!(backups().is_empty());

    let state = test_state(tmp.path());
    let app = build_router(state.clone());
    let events = send(app.clone(), "GET", "/api/raw-events".into(), None).await;
    let preview = events["events"][0]["preview"].as_str().unwrap();
    assert!(preview.starts_with("Bash: curl"), "{preview}");
    assert!(
        !preview.contains("abcd1234efgh5678") && !preview.contains("hunter2pass"),
        "{preview}"
    );

    let hidden = send(
        app.clone(),
        "GET",
        "/api/memories?query=hunter2pass".into(),
        None,
    )
    .await;
    assert_eq!(
        hidden["total"], 0,
        "search index still holds the secret: {hidden}"
    );
    let found = send(
        app.clone(),
        "GET",
        "/api/memories?query=deploy".into(),
        None,
    )
    .await;
    let id = found["memories"][0]["id"].as_str().unwrap().to_string();
    let memory = send(app.clone(), "GET", format!("/api/memories/{id}"), None).await;
    let content = memory["memory"]["content"].as_str().unwrap();
    assert!(
        content.contains("x-api-key: [REDACTED]") && !content.contains("hunter2pass"),
        "{content}"
    );
    let limits = send(
        app.clone(),
        "GET",
        "/api/memories?query=limits".into(),
        None,
    )
    .await;
    let id = limits["memories"][0]["id"].as_str().unwrap().to_string();
    let memory = send(app.clone(), "GET", format!("/api/memories/{id}"), None).await;
    assert_eq!(
        memory["memory"]["content"],
        "Set MAX_TOKEN=4096 for the model"
    );

    let made = backups();
    assert_eq!(made.len(), 1, "{made:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = tokio::fs::metadata(&made[0])
            .await
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    drop((app, state));
    let _again = test_state(tmp.path());
    assert_eq!(backups().len(), 1, "the re-scrub runs once");
}

#[tokio::test]
async fn viewing_connections_writes_only_when_client_state_changes() {
    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());
    let list = || {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri("/api/connections")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            body_json(resp).await["connections"].clone()
        }
    };
    let stamps = || {
        let raw = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
        let mut stmt = raw
            .prepare(
                "SELECT client_id || '=' || updated_at FROM client_connections ORDER BY client_id",
            )
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
    };
    list().await;
    let first = stamps();
    assert_eq!(first.len(), 3);
    list().await;
    assert_eq!(stamps(), first, "an unchanged GET must not rewrite rows");

    // Installing Claude Code changes only its row.
    tokio::fs::create_dir_all(tmp.path().join("home/.claude"))
        .await
        .unwrap();
    let after = list().await;
    let claude = after
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["client_id"] == "claude-code")
        .unwrap();
    assert_eq!(claude["detected"], true, "{claude}");
    let changed: Vec<_> = stamps()
        .into_iter()
        .zip(&first)
        .filter(|(now, before)| now != *before)
        .map(|(now, _)| now)
        .collect();
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert!(changed[0].starts_with("claude-code="));
}

#[tokio::test]
async fn tail_keyset_pages_signal_view_and_preview_only_sse() {
    use axum::{Router, routing::post};

    let mock_router = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            let content = json!({"memories": [{
                "scope": "project", "type": "decision", "title": "Signal tail",
                "summary": "Keyset ordered tail", "content": "Details."
            }]});
            axum::Json(json!({
                "choices": [{ "message": { "content": content.to_string() } }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 4, "cost": 0.0 }
            }))
        }),
    );
    let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock_listener.local_addr().unwrap();
    let mock_task = tokio::spawn(async move {
        let _ = axum::serve(mock_listener, mock_router).await;
    });

    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    tokio::fs::create_dir_all(&repo).await.unwrap();
    let state = test_state(tmp.path());
    let mut settings = state.db.get_settings().unwrap();
    settings.provider = "openai_compatible".into();
    settings.base_url = format!("http://{mock_addr}/v1");
    settings.model = "mock-model".into();
    state.db.save_settings(&settings).unwrap();

    let app = build_router(state.clone());
    let mut sse_rx = state.events_tx.subscribe();

    let hook = |event: &'static str, body: Value| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/v1/hooks/claude?event={event}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    };

    for i in 0..25 {
        hook(
            "UserPromptSubmit",
            json!({"cwd": repo, "session_id": "tail-s1", "prompt": format!("Prompt number {i:02}")}),
        )
        .await;
    }
    let long_cmd = format!("echo {} TAIL_MARKER_END", "x".repeat(3_500));
    hook(
        "PostToolUse",
        json!({"cwd": repo, "session_id": "tail-s1", "tool_name": "Bash", "tool_input": {"command": long_cmd}}),
    )
    .await;
    hook(
        "PostToolUse",
        json!({"cwd": repo, "session_id": "tail-s1", "tool_name": "Edit", "tool_input": {"file_path": "src/main.rs"}}),
    )
    .await;

    // Drain SSE events emitted by hooks and verify preview-only payloads (no payload_json).
    let mut tool_sse = None;
    while let Ok(ev) = sse_rx.try_recv() {
        if ev.kind == "raw_event" {
            assert!(
                ev.payload.get("payload_json").is_none(),
                "SSE leaked payload_json: {}",
                ev.payload
            );
            if ev.payload["tool_name"] == "Bash" {
                tool_sse = Some(ev.payload);
            }
        }
    }
    let tool_sse = tool_sse.expect("expected Bash raw_event on SSE");
    let preview = tool_sse["preview"].as_str().unwrap();
    assert!(preview.chars().count() <= 123, "{preview}");
    assert!(!preview.contains("TAIL_MARKER_END"), "{preview}");

    let proj = ox1_memory::project::resolve_project(&repo);
    let sess = state
        .db
        .upsert_session(&proj.id, "claude-code", "tail-s1", None)
        .unwrap();
    distill_session_once(
        &state.db,
        &state.embedder,
        &state.llm,
        &state.events_tx,
        &sess.id,
    )
    .await;

    let get = |uri: String| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await
        }
    };

    // Signal view excludes tool_use rows: 25 prompts + 1 distill_result = 26 rows.
    let mut seen_ids = Vec::new();
    let mut keys = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let uri = match &cursor {
            Some(c) => format!("/api/raw-events?kind=signal&limit=10&before={c}"),
            None => "/api/raw-events?kind=signal&limit=10".into(),
        };
        let page = get(uri).await;
        assert_eq!(page["total"], 26, "{page}");
        for ev in page["events"].as_array().unwrap() {
            assert_ne!(ev["event_type"], "tool_use");
            seen_ids.push(ev["id"].as_str().unwrap().to_string());
            keys.push((
                ev["created_at"].as_str().unwrap().to_string(),
                ev["id"].as_str().unwrap().to_string(),
            ));
        }
        cursor = page["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen_ids.len(), 26);
    let unique: std::collections::HashSet<&String> = seen_ids.iter().collect();
    assert_eq!(unique.len(), 26, "duplicate event across keyset pages");
    assert!(
        keys.windows(2).all(|w| w[0] >= w[1]),
        "tail not strictly newest-first"
    );

    let all = get("/api/raw-events?kind=all&limit=50".into()).await;
    assert_eq!(all["total"], 28, "{all}");
    let codex_only = get("/api/raw-events?kind=signal&client=codex".into()).await;
    assert_eq!(codex_only["total"], 0, "{codex_only}");

    mock_task.abort();
}

#[tokio::test]
async fn recall_events_recorded_for_briefing_and_prompt_recall() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    tokio::fs::create_dir_all(&repo).await.unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());
    let proj = ox1_memory::project::resolve_project(&repo);
    state.db.upsert_project(&proj).unwrap();

    let create_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/memories")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "scope": "project",
                        "project_id": proj.id,
                        "memory_type": "decision",
                        "title": "Use keyset pagination",
                        "summary": "Keyset cursor prevents duplicate tail rows",
                        "content": "Tail endpoint uses (created_at, id) keyset cursor."
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::OK);

    let hook = |client: &'static str, event: &'static str, body: Value| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/v1/hooks/{client}?event={event}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    };

    hook(
        "claude",
        "SessionStart",
        json!({"cwd": repo, "session_id": "rec-c1"}),
    )
    .await;
    // Short prompt (<12 chars) should not trigger recall.
    hook(
        "claude",
        "UserPromptSubmit",
        json!({"cwd": repo, "session_id": "rec-c1", "prompt": "hi"}),
    )
    .await;
    // Matching prompt (>=12 chars) triggers prompt recall.
    hook(
        "codex",
        "UserPromptSubmit",
        json!({"cwd": repo, "session_id": "rec-x1", "prompt": "How does keyset pagination work?"}),
    )
    .await;

    let get = |uri: &'static str| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await
        }
    };

    let recalls = get("/api/raw-events?kind=recall").await;
    assert_eq!(recalls["total"], 2, "{recalls}");
    let rows = recalls["events"].as_array().unwrap();
    assert!(rows.iter().all(|r| r["event_type"] == "recall"));
    assert!(rows.iter().all(|r| r["distilled"] == true));
    assert_eq!(rows[0]["preview"], "Prompt recall · 1 memory");
    assert_eq!(rows[0]["source_client"], "codex");
    assert_eq!(rows[1]["preview"], "Briefing · 1 memory");
    assert_eq!(rows[1]["source_client"], "claude-code");

    let codex_recalls = get("/api/raw-events?kind=recall&client=codex").await;
    assert_eq!(codex_recalls["total"], 1, "{codex_recalls}");
    let signal = get("/api/raw-events?kind=signal").await;
    assert!(
        signal["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["event_type"] == "recall")
            .count()
            == 2
    );
}

#[tokio::test]
async fn handoff_transfers_recent_sessions_and_respects_guards() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    tokio::fs::create_dir_all(&repo).await.unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());

    let post_hook = |client: &'static str, event: &'static str, body: Value| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/v1/hooks/{client}?event={event}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await
        }
    };

    // 1. Claude Code (inside T3 Code) runs a session with a credential + <private> block.
    post_hook(
        "claude",
        "UserPromptSubmit",
        json!({
            "cwd": repo,
            "session_id": "cl-1",
            "ox1_host": "t3-code",
            "prompt": "Fix HF_TOKEN=hf_abcdefghijklmnopqrstu <private>personal secret note</private> in auth"
        }),
    )
    .await;
    post_hook(
        "claude",
        "UserPromptSubmit",
        json!({
            "cwd": repo,
            "session_id": "cl-1",
            "ox1_host": "t3-code",
            "prompt": "Switch session state to keyset cursor"
        }),
    )
    .await;

    let proj = ox1_memory::project::resolve_project(&repo);

    // 2. source: clear, compact, resume give NO Handoff (tested before startup drains pending events).
    for src in ["clear", "compact", "resume"] {
        let skipped = post_hook(
            "antigravity",
            "SessionStart",
            json!({"cwd": repo, "conversationId": format!("agy-{src}"), "source": src}),
        )
        .await;
        assert!(
            skipped.get("injectSteps").is_none(),
            "source={src} should not receive Handoff: {skipped}"
        );
    }

    // 3. Antigravity starts fresh (no source) -> receives Handoff with redacted prompt & undistilled count.
    let agy = post_hook(
        "antigravity",
        "SessionStart",
        json!({"cwd": repo, "conversationId": "agy-fresh"}),
    )
    .await;
    let ctx = agy["injectSteps"][0]["systemMessage"]["systemMessage"]
        .as_str()
        .unwrap();
    assert!(ctx.contains("Generated at "), "{ctx}");
    assert!(ctx.contains("## Handoff"), "{ctx}");
    assert!(ctx.contains("Claude Code via T3 Code"), "{ctx}");
    assert!(
        ctx.contains("Switch session state to keyset cursor"),
        "{ctx}"
    );
    assert!(ctx.contains("[REDACTED]"), "{ctx}");
    assert!(!ctx.contains("hf_abcdefghijklmnopqrstu"), "{ctx}");
    assert!(!ctx.contains("personal secret note"), "{ctx}");
    assert!(ctx.contains("2 undistilled events"), "{ctx}");

    // Recall row records Handoff session ids and renders "picked up from <Client> via <Host>".
    let recalls = body_json(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/raw-events?kind=recall")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(recalls["total"], 1, "{recalls}");
    assert_eq!(
        recalls["events"][0]["preview"],
        "picked up from Claude Code via T3 Code"
    );
    let cl_sess = state
        .db
        .upsert_session(&proj.id, "claude-code", "cl-1", None)
        .unwrap();
    let raw_db = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
    let recall_payload: String = raw_db
        .query_row(
            "SELECT payload_json FROM raw_events WHERE event_type = 'recall' ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(recall_payload.contains(&cl_sess.id), "{recall_payload}");

    // 4. Paused project gives no Handoff.
    state.db.set_project_privacy(&proj.id, "paused").unwrap();
    let paused = post_hook(
        "codex",
        "SessionStart",
        json!({"cwd": repo, "session_id": "cdx-paused"}),
    )
    .await;
    assert!(paused.get("hookSpecificOutput").is_none(), "{paused}");

    // 5. Cap of 3 concurrent recent sessions + 12h window exclusion on a second project.
    let repo2 = tmp.path().join("repo2");
    tokio::fs::create_dir_all(&repo2).await.unwrap();
    for (idx, ext_id, prompt) in [
        (0, "s-old", "Very old session prompt"),
        (1, "s-1", "First recent prompt"),
        (2, "s-2", "Second recent prompt"),
        (3, "s-3", "Third recent prompt"),
        (4, "s-4", "Fourth recent prompt"),
    ] {
        post_hook(
            "claude",
            "UserPromptSubmit",
            json!({"cwd": repo2, "session_id": ext_id, "prompt": prompt}),
        )
        .await;
        let ts = if idx == 0 {
            (chrono::Utc::now() - chrono::Duration::hours(13)).to_rfc3339()
        } else {
            (chrono::Utc::now() - chrono::Duration::minutes(10 - idx as i64)).to_rfc3339()
        };
        raw_db
            .execute(
                "UPDATE sessions SET last_turn_at = ?1 WHERE external_session_id = ?2",
                rusqlite::params![ts, ext_id],
            )
            .unwrap();
    }
    let cdx = post_hook(
        "codex",
        "SessionStart",
        json!({"cwd": repo2, "session_id": "cdx-cap", "source": "startup"}),
    )
    .await;
    let ctx2 = cdx["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(!ctx2.contains("Very old session prompt"), "{ctx2}");
    assert!(!ctx2.contains("First recent prompt"), "{ctx2}");
    assert!(ctx2.contains("Second recent prompt"), "{ctx2}");
    assert!(ctx2.contains("Third recent prompt"), "{ctx2}");
    assert!(ctx2.contains("Fourth recent prompt"), "{ctx2}");
}

#[tokio::test]
async fn recent_sort_orders_by_recency_without_pin_priority() {
    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());

    let post_mem = |title: &'static str, status: &'static str| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/memories")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({
                                "scope": "global",
                                "memory_type": "decision",
                                "title": title,
                                "summary": "Order check",
                                "content": "Details",
                                "status": status
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await["memory"]["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };

    let older_pinned = post_mem("Older pinned rule", "pinned").await;
    let raw = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
    let older_ts = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    raw.execute(
        "UPDATE memories SET updated_at = ?1 WHERE id = ?2",
        rusqlite::params![older_ts, older_pinned],
    )
    .unwrap();
    let newer_active = post_mem("Newer active rule", "active").await;

    let get = |uri: &'static str| {
        let app = app.clone();
        async move {
            let resp = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await
        }
    };

    let recent = get("/api/memories?sort=recent").await;
    let r_mems = recent["memories"].as_array().unwrap();
    assert_eq!(r_mems[0]["id"], newer_active);
    assert_eq!(r_mems[1]["id"], older_pinned);

    let default_sort = get("/api/memories").await;
    let d_mems = default_sort["memories"].as_array().unwrap();
    assert_eq!(d_mems[0]["id"], older_pinned);
    assert_eq!(d_mems[1]["id"], newer_active);
}

#[tokio::test]
async fn usage_summary_counts_only_today_via_range_predicate() {
    let tmp = tempdir().unwrap();
    let state = test_state(tmp.path());
    let app = build_router(state.clone());

    state
        .db
        .record_usage(
            "openai_compatible",
            "m-old",
            "distill",
            500,
            100,
            0.25,
            None,
            "ok",
            None,
        )
        .unwrap();
    state
        .db
        .record_usage(
            "openai_compatible",
            "m-now",
            "distill",
            120,
            30,
            0.05,
            None,
            "ok",
            None,
        )
        .unwrap();

    let raw = rusqlite::Connection::open(tmp.path().join("memory.db")).unwrap();
    let yesterday = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    raw.execute(
        "UPDATE usage_ledger SET created_at = ?1 WHERE model = 'm-old'",
        rusqlite::params![yesterday],
    )
    .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/summary")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let summary = body_json(resp).await;
    assert_eq!(summary["today_input_tokens"], 120, "{summary}");
    assert_eq!(summary["today_output_tokens"], 30, "{summary}");
    assert_eq!(summary["today_total_tokens"], 150, "{summary}");
    assert_eq!(summary["today_calls"], 1, "{summary}");
    let cost = summary["today_cost_usd"].as_f64().unwrap();
    assert!((cost - 0.05).abs() < 1e-6, "{cost}");
}
