use crate::db::MemoryDb;
use crate::distill::DistillHandle;
use crate::embed::Embedder;
use crate::models::{SearchFilter, StreamEvent, normalize_client};
use crate::project::resolve_project;
use crate::scrub::scrub_json_value;
use chrono::Utc;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// Deduplicates Antigravity `PreInvocation` (which fires on every model step inside a turn).
#[derive(Clone, Default)]
pub struct HookState {
    agy_last_prompt: Arc<Mutex<HashMap<String, String>>>,
}

impl HookState {
    pub fn should_inject_agy_prompt(&self, conv_id: &str, prompt: &str) -> bool {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            return false;
        }
        let mut map = self.agy_last_prompt.lock().unwrap();
        if map.len() > 256 {
            map.clear();
        }
        if map.get(conv_id).map(|s| s.as_str()) == Some(trimmed) {
            return false;
        }
        map.insert(conv_id.to_string(), trimmed.to_string());
        true
    }
}

/// Fast (<50ms) hook handler for Claude Code, Codex CLI/App, and Antigravity.
/// Captures raw events at 0 LLM tokens and triggers async per-turn distillation on Stop/PreCompact.
#[allow(clippy::too_many_arguments)]
pub async fn handle_hook(
    client_param: &str,
    event_override: Option<&str>,
    payload: Value,
    db: &MemoryDb,
    embedder: &Embedder,
    distill: &DistillHandle,
    events_tx: &broadcast::Sender<StreamEvent>,
    hook_state: &HookState,
) -> Value {
    let client = normalize_client(Some(client_param));
    let _ = db.touch_client_activity(&client, true);

    let event_name = event_override
        .filter(|s| !s.trim().is_empty())
        .or_else(|| payload.get("hook_event_name").and_then(|v| v.as_str()))
        .or_else(|| payload.get("hookEventName").and_then(|v| v.as_str()))
        .unwrap_or("Unknown")
        .to_string();

    let cwd = extract_cwd(&payload);
    let project_ident = resolve_project(Path::new(&cwd));
    let project = match db.upsert_project(&project_ident) {
        Ok(p) => p,
        Err(_) => return json!({}),
    };

    let ext_session_id = extract_session_id(&payload, &project.id);
    let session = match db.upsert_session(
        &project.id,
        &client,
        &ext_session_id,
        project_ident.branch.as_deref(),
    ) {
        Ok(s) => s,
        Err(_) => return json!({}),
    };

    let settings = db.get_settings().unwrap_or_default();
    let is_paused = project.privacy_mode == "paused";

    match event_name.as_str() {
        "SessionStart" => {
            let (detail, included_ids) = db
                .build_session_briefing_detailed(
                    Some(&project.id),
                    settings.session_start_token_budget,
                )
                .unwrap_or_default();
            if detail.briefing.is_empty() {
                return json!({});
            }
            let _ = db.record_memory_recalls(&included_ids, &client);
            format_context_injection(
                &client,
                "SessionStart",
                &detail.briefing,
                detail.memory_count,
            )
        }
        "UserPromptSubmit" | "PreInvocation" => {
            let prompt = payload
                .get("prompt")
                .or_else(|| payload.get("lastUserInput"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Antigravity PreInvocation fires before every tool-loop model call — deduplicate per turn!
            if client == "antigravity"
                && event_name == "PreInvocation"
                && !hook_state.should_inject_agy_prompt(&ext_session_id, prompt)
            {
                return json!({});
            }

            if !is_paused && !prompt.trim().is_empty() {
                let clean = scrub_json_value(&json!({ "prompt": prompt }), 2_000);
                if let Ok(ev) = db.insert_raw_event(
                    &session.id,
                    &project.id,
                    &client,
                    "prompt",
                    None,
                    &clean.to_string(),
                ) {
                    let _ = events_tx.send(StreamEvent {
                        kind: "raw_event".into(),
                        timestamp: Utc::now().to_rfc3339(),
                        payload: serde_json::to_value(&ev).unwrap_or_default(),
                    });
                }
            }

            // Ultra-fast targeted recall (FTS5 + local ONNX if already loaded; never blocks)
            if prompt.trim().len() >= 12 {
                let q_vec = if settings.embedding_mode == "local_onnx" {
                    embedder.embed_text(prompt, &settings, None, false).await
                } else {
                    None
                };
                if let Ok(hits) = db.search_memories(
                    &SearchFilter {
                        query: Some(prompt.chars().take(300).collect()),
                        project_id: Some(project.id.clone()),
                        limit: Some(3),
                        ..Default::default()
                    },
                    q_vec.as_deref(),
                ) && !hits.is_empty()
                {
                    let ids: Vec<String> = hits.iter().map(|h| h.id.clone()).collect();
                    let _ = db.record_memory_recalls(&ids, &client);
                    let mut lines = String::from(
                        "Relevant cross-agent memories (call `memory_get([id])` if needed):\n",
                    );
                    for h in &hits {
                        lines.push_str(&format!(
                            "- [{}|{}] {}: {} (id:{})\n",
                            h.source_client, h.memory_type, h.title, h.summary, h.id
                        ));
                    }
                    return format_context_injection(
                        &client,
                        &event_name,
                        lines.trim(),
                        hits.len(),
                    );
                }
            }
            json!({})
        }
        "PostToolUse" | "PostToolUseFailure" => {
            if is_paused {
                return json!({});
            }
            let tool_name = payload
                .get("tool_name")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    payload
                        .get("toolCall")
                        .and_then(|tc| tc.get("name"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("unknown");

            // Skip our own MCP memory tools and trivial read-only noise to keep signal pure
            if tool_name.starts_with("mcp__0x1")
                || tool_name.contains("memory_search")
                || tool_name.contains("memory_get")
                || matches!(
                    tool_name,
                    "Read" | "Glob" | "Grep" | "LS" | "read_file" | "list_dir"
                )
            {
                return json!({});
            }

            let ev_type = if event_name == "PostToolUseFailure"
                || json_error_is_present(payload.get("error"))
            {
                "tool_error"
            } else {
                "tool_use"
            };
            let compact_payload = scrub_json_value(
                &json!({
                    "tool": tool_name,
                    "input": payload.get("tool_input").or_else(|| payload.get("toolCall").and_then(|c| c.get("args"))),
                    "output": payload.get("tool_response").or_else(|| payload.get("tool_output")).or_else(|| payload.get("error")),
                }),
                1_200,
            );
            if let Ok(ev) = db.insert_raw_event(
                &session.id,
                &project.id,
                &client,
                ev_type,
                Some(tool_name),
                &compact_payload.to_string(),
            ) {
                let _ = events_tx.send(StreamEvent {
                    kind: "raw_event".into(),
                    timestamp: Utc::now().to_rfc3339(),
                    payload: serde_json::to_value(&ev).unwrap_or_default(),
                });
            }
            json!({})
        }
        "Stop" | "PreCompact" | "SessionEnd" => {
            if !is_paused {
                if let Some(last_msg) = payload
                    .get("last_assistant_message")
                    .or_else(|| payload.get("lastAssistantMessage"))
                    .and_then(|v| v.as_str())
                    && let Ok(ev) = db.insert_raw_event(
                        &session.id,
                        &project.id,
                        &client,
                        "turn_stop",
                        None,
                        &scrub_json_value(&json!({ "assistant": last_msg }), 2_000).to_string(),
                    )
                {
                    let _ = events_tx.send(StreamEvent {
                        kind: "raw_event".into(),
                        timestamp: Utc::now().to_rfc3339(),
                        payload: serde_json::to_value(&ev).unwrap_or_default(),
                    });
                }
                // Trigger async 1x per-turn distillation off the hot path (<5ms hook return!)
                distill.queue_session(session.id.clone());
            }
            if event_name == "SessionEnd" {
                let _ = db.end_session(&session.id);
            }
            json!({})
        }
        _ => json!({}),
    }
}

fn json_error_is_present(val: Option<&Value>) -> bool {
    match val {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Number(_)) => true,
    }
}

fn extract_cwd(payload: &Value) -> String {
    payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .or_else(|| {
            payload
                .get("workspacePaths")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
        })
        .filter(|s| !s.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".into())
        })
}

fn extract_session_id(payload: &Value, project_id: &str) -> String {
    payload
        .get("session_id")
        .or_else(|| payload.get("conversationId"))
        .or_else(|| payload.get("thread_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("ephemeral_{project_id}"))
}

fn format_context_injection(client: &str, event_name: &str, context: &str, count: usize) -> Value {
    match client {
        "antigravity" => json!({
            "injectSteps": [{
                "systemMessage": {
                    "systemMessage": context,
                    "metadata": {
                        "user_markdown": format!("*0x1-memory: loaded {count} cross-agent memories*")
                    }
                }
            }]
        }),
        // Codex: NEVER emit top-level `systemMessage` (renders as yellow warning badge in TUI).
        "codex" => json!({
            "hookSpecificOutput": {
                "hookEventName": event_name,
                "additionalContext": context
            }
        }),
        // Claude Code
        _ => json!({
            "hookSpecificOutput": {
                "hookEventName": event_name,
                "additionalContext": context
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distill::spawn_distill_worker;
    use crate::llm::MemoryLlmClient;
    use crate::models::NewMemoryInput;
    use tempfile::tempdir;

    #[tokio::test]
    async fn codex_never_emits_system_message_and_agy_uses_inject_steps() {
        let db = MemoryDb::open_in_memory().unwrap();
        let mut s = db.get_settings().unwrap();
        s.embedding_mode = "fts_only".into();
        db.save_settings(&s).unwrap();
        let tmp = tempdir().unwrap();
        let embedder = Embedder::new(tmp.path().to_path_buf());
        let (tx, _rx) = broadcast::channel(16);
        let distill = spawn_distill_worker(
            db.clone(),
            embedder.clone(),
            MemoryLlmClient::new(),
            tx.clone(),
        );
        let state = HookState::default();

        db.create_memory(
            NewMemoryInput {
                scope: Some("global".into()),
                project_id: None,
                session_id: None,
                source_client: Some("claude-code".into()),
                memory_type: "preference".into(),
                title: "Use concise Indonesian replies".into(),
                summary: "Prefer Bahasa Indonesia in explanations".into(),
                content: "User prefers casual Indonesian".into(),
                files: None,
                tags: None,
                status: Some("pinned".into()),
                supersedes_id: None,
            },
            None,
        )
        .unwrap();

        let codex_out = handle_hook(
            "codex",
            Some("SessionStart"),
            json!({"cwd": tmp.path(), "session_id": "codex-1"}),
            &db,
            &embedder,
            &distill,
            &tx,
            &state,
        )
        .await;
        assert!(codex_out.get("systemMessage").is_none());
        assert!(
            codex_out["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains("Indonesian")
        );

        let agy_out = handle_hook(
            "antigravity",
            Some("SessionStart"),
            json!({"cwd": tmp.path(), "conversationId": "agy-1"}),
            &db,
            &embedder,
            &distill,
            &tx,
            &state,
        )
        .await;
        assert!(
            agy_out["injectSteps"][0]["systemMessage"]["systemMessage"]
                .as_str()
                .unwrap()
                .contains("Indonesian")
        );

        // PreInvocation deduplication inside same turn
        let p1 = handle_hook(
            "antigravity",
            Some("PreInvocation"),
            json!({"cwd": tmp.path(), "conversationId": "agy-1", "lastUserInput": "Explain Indonesian preference"}),
            &db,
            &embedder,
            &distill,
            &tx,
            &state,
        )
        .await;
        let p2 = handle_hook(
            "antigravity",
            Some("PreInvocation"),
            json!({"cwd": tmp.path(), "conversationId": "agy-1", "lastUserInput": "Explain Indonesian preference"}),
            &db,
            &embedder,
            &distill,
            &tx,
            &state,
        )
        .await;
        assert!(p1.get("injectSteps").is_some());
        assert!(p2.get("injectSteps").is_none());

        // PostToolUse with null or empty error records tool_use (never tool_error)
        let proj = crate::project::resolve_project(tmp.path());
        handle_hook(
            "antigravity",
            Some("PostToolUse"),
            json!({"cwd": tmp.path(), "conversationId": "agy-1", "tool_name": "run_command", "error": null}),
            &db,
            &embedder,
            &distill,
            &tx,
            &state,
        )
        .await;
        handle_hook(
            "antigravity",
            Some("PostToolUse"),
            json!({"cwd": tmp.path(), "conversationId": "agy-1", "tool_name": "run_command", "error": ""}),
            &db,
            &embedder,
            &distill,
            &tx,
            &state,
        )
        .await;
        handle_hook(
            "antigravity",
            Some("PostToolUse"),
            json!({"cwd": tmp.path(), "conversationId": "agy-1", "tool_name": "run_command", "error": "exit code 1"}),
            &db,
            &embedder,
            &distill,
            &tx,
            &state,
        )
        .await;
        let recent = db.recent_raw_events(Some(&proj.id), None, 10).unwrap();
        let err_count = recent
            .iter()
            .filter(|e| e.event_type == "tool_error")
            .count();
        let ok_count = recent.iter().filter(|e| e.event_type == "tool_use").count();
        assert_eq!(err_count, 1);
        assert_eq!(ok_count, 2);
    }
}
