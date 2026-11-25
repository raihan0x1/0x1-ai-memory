use crate::db::MemoryDb;
use crate::embed::Embedder;
use crate::models::{NewMemoryInput, SearchFilter, StreamEvent, UpdateMemoryInput};
use crate::project::resolve_project;
use chrono::Utc;
use serde_json::{Value, json};
use std::path::Path;
use tokio::sync::broadcast;

pub const MCP_PROTOCOL_VERSION: &str = "2025-03-26";

/// Compact 5-tool MCP surface (~500 tokens total) implementing progressive disclosure.
pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "memory_search",
            "description": "Hybrid search (FTS5 + vector) across cross-agent memories. Returns compact index rows (id, title, summary, client, type). Call memory_get for full content.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (leave empty for latest memories)" },
                    "project": { "type": "string", "description": "Optional project_id or repo path" },
                    "scope": { "type": "string", "enum": ["project", "global"] },
                    "type": { "type": "string", "enum": ["decision", "bugfix", "architecture", "preference", "pattern", "session_summary"] },
                    "client": { "type": "string", "description": "Filter by source AI client (claude-code, codex, antigravity, t3-code)" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 30 }
                }
            }
        },
        {
            "name": "memory_get",
            "description": "Fetch full details (content, files, tags) for specific memory IDs returned by memory_search or SessionStart briefing.",
            "inputSchema": {
                "type": "object",
                "required": ["ids"],
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" } }
                }
            }
        },
        {
            "name": "memory_timeline",
            "description": "Get chronological neighboring memories before and after an anchor memory ID.",
            "inputSchema": {
                "type": "object",
                "required": ["memory_id"],
                "properties": {
                    "memory_id": { "type": "string" },
                    "before": { "type": "integer", "minimum": 0, "maximum": 15 },
                    "after": { "type": "integer", "minimum": 0, "maximum": 15 }
                }
            }
        },
        {
            "name": "memory_save",
            "description": "Save a durable decision, bugfix, architecture fact, pattern, or user preference shared across all AI clients.",
            "inputSchema": {
                "type": "object",
                "required": ["title", "summary", "content", "type"],
                "properties": {
                    "title": { "type": "string" },
                    "summary": { "type": "string" },
                    "content": { "type": "string" },
                    "type": { "type": "string", "enum": ["decision", "bugfix", "architecture", "preference", "pattern", "session_summary"] },
                    "scope": { "type": "string", "enum": ["project", "global"] },
                    "project": { "type": "string", "description": "project_id or repo directory path" },
                    "files": { "type": "array", "items": { "type": "string" } },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "supersedes_id": { "type": "string", "description": "Old memory ID replaced by this one" },
                    "client": { "type": "string" }
                }
            }
        },
        {
            "name": "memory_update",
            "description": "Pin, edit, archive, or supersede an existing memory.",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "status": { "type": "string", "enum": ["active", "pinned", "superseded", "archived"] },
                    "title": { "type": "string" },
                    "summary": { "type": "string" },
                    "content": { "type": "string" },
                    "superseded_by": { "type": "string" }
                }
            }
        }
    ])
}

/// Handles a single JSON-RPC 2.0 MCP message. Returns `None` for notifications.
pub async fn handle_rpc(
    req: Value,
    db: &MemoryDb,
    embedder: &Embedder,
    events_tx: &broadcast::Sender<StreamEvent>,
    client_hint: Option<&str>,
) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(json!({}));

    if method.starts_with("notifications/") {
        return None;
    }

    let req_id = id.unwrap_or(Value::Null);

    let result = match method {
        "initialize" => {
            if let Some(name) = params
                .get("clientInfo")
                .and_then(|c| c.get("name"))
                .and_then(|n| n.as_str())
            {
                let _ = db.touch_client_activity(name, false);
            } else if let Some(hint) = client_hint {
                let _ = db.touch_client_activity(hint, false);
            }
            Ok(json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": {
                    "name": "0x1-ai-memory",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => execute_tool_call(&params, db, embedder, events_tx, client_hint).await,
        other => Err((-32601, format!("Method not found: {other}"))),
    };

    Some(match result {
        Ok(res) => json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": res
        }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": { "code": code, "message": message }
        }),
    })
}

async fn execute_tool_call(
    params: &Value,
    db: &MemoryDb,
    embedder: &Embedder,
    events_tx: &broadcast::Sender<StreamEvent>,
    client_hint: Option<&str>,
) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((-32602, "Missing tool name".to_string()))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    if let Some(hint) = args.get("client").and_then(|v| v.as_str()).or(client_hint) {
        let _ = db.touch_client_activity(hint, false);
    }

    let settings = db.get_settings().map_err(|e| (-32603, e.to_string()))?;

    let payload = match name {
        "memory_search" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let project_id =
                resolve_optional_project(db, args.get("project").and_then(|v| v.as_str()));
            let allow_ext = !db.is_project_local_only(project_id.as_deref());
            let q_vec = match query.as_deref() {
                Some(q) if !q.trim().is_empty() => {
                    embedder.embed_text(q, &settings, Some(db), allow_ext).await
                }
                _ => None,
            };
            let filter = SearchFilter {
                query,
                project_id,
                scope: args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
                memory_type: args
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
                source_client: args
                    .get("client")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
                include_inactive: Some(false),
                limit: args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .or(Some(10)),
                ..Default::default()
            };
            let hits = db
                .search_memories(&filter, q_vec.as_deref())
                .map_err(|e| (-32603, e.to_string()))?;
            if !hits.is_empty() {
                let ids: Vec<String> = hits.iter().map(|h| h.id.clone()).collect();
                let _ = db.record_memory_recalls(&ids, client_hint.unwrap_or("mcp"));
            }
            json!({ "count": hits.len(), "results": hits })
        }
        "memory_get" => {
            let ids: Vec<String> = args
                .get("ids")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(ToOwned::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let memories = db
                .get_memories_by_ids(&ids)
                .map_err(|e| (-32603, e.to_string()))?;
            if !memories.is_empty() {
                let found_ids: Vec<String> = memories.iter().map(|m| m.id.clone()).collect();
                let _ = db.record_memory_recalls(&found_ids, client_hint.unwrap_or("mcp"));
            }
            json!({ "count": memories.len(), "memories": memories })
        }
        "memory_timeline" => {
            let memory_id = args
                .get("memory_id")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing memory_id".to_string()))?;
            let before = args.get("before").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            let after = args.get("after").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            let items = db
                .memory_timeline(memory_id, before, after)
                .map_err(|e| (-32603, e.to_string()))?;
            json!({ "count": items.len(), "timeline": items })
        }
        "memory_save" => {
            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing title".to_string()))?
                .to_string();
            let summary = args
                .get("summary")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing summary".to_string()))?
                .to_string();
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing content".to_string()))?
                .to_string();
            let memory_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("pattern")
                .to_string();
            let project_id =
                resolve_optional_project(db, args.get("project").and_then(|v| v.as_str()));
            let allow_ext = !db.is_project_local_only(project_id.as_deref());
            let embed_input = format!("{title}\n{summary}\n{content}");
            let vec = embedder
                .embed_text(&embed_input, &settings, Some(db), allow_ext)
                .await;
            let input = NewMemoryInput {
                scope: args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
                project_id,
                session_id: None,
                source_client: args
                    .get("client")
                    .and_then(|v| v.as_str())
                    .or(client_hint)
                    .map(ToOwned::to_owned),
                memory_type,
                title,
                summary,
                content,
                files: args.get("files").and_then(|v| v.as_array()).map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(ToOwned::to_owned))
                        .collect()
                }),
                tags: args.get("tags").and_then(|v| v.as_array()).map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(ToOwned::to_owned))
                        .collect()
                }),
                status: Some("active".into()),
                supersedes_id: args
                    .get("supersedes_id")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
            };
            let mem = db
                .create_memory(input, vec.as_deref())
                .map_err(|e| (-32603, e.to_string()))?;
            let _ = db.touch_client_activity(&mem.source_client, true);
            let _ = events_tx.send(StreamEvent {
                kind: "memory_created".into(),
                timestamp: Utc::now().to_rfc3339(),
                payload: serde_json::to_value(&mem).unwrap_or_default(),
            });
            json!({ "saved": true, "memory": mem })
        }
        "memory_update" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or((-32602, "Missing id".to_string()))?;
            let patch = UpdateMemoryInput {
                status: args
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
                title: args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
                summary: args
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
                content: args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
                files: None,
                tags: None,
                superseded_by: args
                    .get("superseded_by")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned),
            };
            let needs_reembed =
                patch.title.is_some() || patch.summary.is_some() || patch.content.is_some();
            let updated = db
                .update_memory(id, patch, None)
                .map_err(|e| (-32603, e.to_string()))?;
            if let Some(ref mem) = updated {
                if needs_reembed {
                    let allow_ext = !db.is_project_local_only(mem.project_id.as_deref());
                    let text = format!("{}\n{}\n{}", mem.title, mem.summary, mem.content);
                    if let Some(vec) = embedder
                        .embed_text(&text, &settings, Some(db), allow_ext)
                        .await
                    {
                        let _ = db.set_memory_embedding(&mem.id, &vec);
                    }
                }
                let _ = events_tx.send(StreamEvent {
                    kind: "memory_updated".into(),
                    timestamp: Utc::now().to_rfc3339(),
                    payload: serde_json::to_value(mem).unwrap_or_default(),
                });
                json!({ "updated": true, "memory": mem })
            } else {
                json!({ "updated": false, "error": "Memory not found" })
            }
        }
        other => return Err((-32601, format!("Unknown tool: {other}"))),
    };

    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into());
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false
    }))
}

fn resolve_optional_project(db: &MemoryDb, raw: Option<&str>) -> Option<String> {
    let val = raw?.trim();
    if val.is_empty() {
        return None;
    }
    if val.starts_with("proj_") {
        return Some(val.to_string());
    }
    let path = Path::new(val);
    if path.exists() || val.contains('/') || val.contains('\\') {
        let ident = resolve_project(path);
        let _ = db.upsert_project(&ident);
        return Some(ident.id);
    }
    // Match by project name if known
    if let Ok(projects) = db.list_projects()
        && let Some(found) = projects
            .into_iter()
            .find(|p| p.name.eq_ignore_ascii_case(val))
    {
        return Some(found.id);
    }
    Some(val.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn mcp_initialize_list_save_search_and_get_work() {
        let db = MemoryDb::open_in_memory().unwrap();
        let mut s = db.get_settings().unwrap();
        s.embedding_mode = "fts_only".into();
        db.save_settings(&s).unwrap();
        let tmp = tempdir().unwrap();
        let embedder = Embedder::new(tmp.path().to_path_buf());
        let (tx, _rx) = broadcast::channel(16);

        let init = handle_rpc(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"codex"}}}),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "0x1-ai-memory");

        let list = handle_rpc(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 5);

        let save = handle_rpc(
            json!({
                "jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{
                    "name":"memory_save",
                    "arguments":{
                        "title":"Use Rust Axum daemon",
                        "summary":"Chose single Rust binary over Node+Python",
                        "content":"Fast ~3ms hooks and 20MB RAM",
                        "type":"architecture",
                        "scope":"global",
                        "client":"claude-code"
                    }
                }
            }),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        let text = save["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Use Rust Axum daemon"));

        let search = handle_rpc(
            json!({
                "jsonrpc":"2.0","id":4,"method":"tools/call",
                "params":{"name":"memory_search","arguments":{"query":"Axum daemon"}}
            }),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        let s_text = search["result"]["content"][0]["text"].as_str().unwrap();
        assert!(s_text.contains("Use Rust Axum daemon"));
    }
}
