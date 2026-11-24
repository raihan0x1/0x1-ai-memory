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
    let mut s = db.get_settings().unwrap();
    s.embedding_mode = "fts_only".into();
    db.save_settings(&s).unwrap();
    let embedder = Embedder::new(tmp.join("models"));
    let llm = MemoryLlmClient::new();
    let (events_tx, _) = broadcast::channel::<StreamEvent>(32);
    let distill =
        spawn_distill_worker(db.clone(), embedder.clone(), llm.clone(), events_tx.clone());
    let connectors = ConnectorManager::new(
        tmp.join("home"),
        tmp.join("backups"),
        PathBuf::from("/usr/local/bin/0x1-memory"),
        41777,
    );
    AppState {
        db,
        embedder,
        llm,
        distill,
        connectors,
        events_tx,
        hook_state: HookState::default(),
        port: 41777,
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
}
