use crate::clock::{generated_at, memory_when};
use crate::db::MemoryDb;
use crate::embed::Embedder;
use crate::models::{NewMemoryInput, SearchFilter, StreamEvent, UpdateMemoryInput};
use crate::project::resolve_project;
use chrono::{Local, Utc};
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
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (leave empty for latest memories)" },
                    "project": { "type": "string", "description": "Optional project_id or repo path" },
                    "scope": { "type": "string", "enum": ["project", "global"] },
                    "type": { "type": "string", "enum": ["decision", "bugfix", "architecture", "preference", "pattern", "session_summary"] },
                    "client": { "type": "string", "description": "Filter by source AI client (claude-code, codex, antigravity, mcp)" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 30 }
                }
            }
        },
        {
            "name": "memory_get",
            "description": "Fetch full details (content, files, tags) for specific memory IDs returned by memory_search or SessionStart briefing.",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            },
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
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            },
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
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": false
            },
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
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": true,
                "openWorldHint": false
            },
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
            let now = Local::now();
            let results: Vec<Value> = hits
                .iter()
                .map(|h| {
                    let mut v = serde_json::to_value(h).unwrap_or_default();
                    v["updated"] = memory_when(&h.updated_at, &now, "this search").into();
                    v
                })
                .collect();
            json!({ "generated_at": generated_at(&now), "count": hits.len(), "results": results })
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

    fn test_env() -> (
        MemoryDb,
        tempfile::TempDir,
        Embedder,
        broadcast::Sender<StreamEvent>,
    ) {
        let db = MemoryDb::open_in_memory().unwrap();
        let mut s = db.get_settings().unwrap();
        s.embedding_mode = "fts_only".into();
        db.save_settings(&s).unwrap();
        let tmp = tempdir().unwrap();
        let embedder = Embedder::new(tmp.path().to_path_buf());
        let (tx, _rx) = broadcast::channel(16);
        (db, tmp, embedder, tx)
    }

    fn tool_payload(resp: &Value) -> Value {
        assert_eq!(resp["result"]["isError"], json!(false), "{resp}");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    #[tokio::test]
    async fn mcp_tools_list_declares_all_five_tools_and_annotations() {
        let (db, _tmp, embedder, tx) = test_env();

        let list = handle_rpc(
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 5);

        let expected = [
            ("memory_search", true, false, true, false),
            ("memory_get", true, false, true, false),
            ("memory_timeline", true, false, true, false),
            ("memory_save", false, false, false, false),
            ("memory_update", false, true, true, false),
        ];

        for (name, read_only, destructive, idempotent, open_world) in expected {
            let tool = tools
                .iter()
                .find(|t| t["name"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("missing tool {name} in tools/list"));
            let ann = tool["annotations"]
                .as_object()
                .unwrap_or_else(|| panic!("missing annotations object for {name}"));
            assert_eq!(
                ann.get("readOnlyHint").and_then(Value::as_bool),
                Some(read_only),
                "{name}.readOnlyHint"
            );
            assert_eq!(
                ann.get("destructiveHint").and_then(Value::as_bool),
                Some(destructive),
                "{name}.destructiveHint"
            );
            assert_eq!(
                ann.get("idempotentHint").and_then(Value::as_bool),
                Some(idempotent),
                "{name}.idempotentHint"
            );
            assert_eq!(
                ann.get("openWorldHint").and_then(Value::as_bool),
                Some(open_world),
                "{name}.openWorldHint"
            );
        }
    }

    #[tokio::test]
    async fn mcp_initialize_and_all_five_tools_work() {
        let (db, _tmp, embedder, tx) = test_env();

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

        // 1. memory_save (save three memories to exercise search, get, timeline, and update)
        let mut saved_ids = Vec::new();
        for (idx, (title, summary, content)) in [
            (
                "Use Rust Axum daemon",
                "Chose single Rust binary over Node+Python",
                "Fast ~3ms hooks and 20MB RAM",
            ),
            (
                "Store memories in SQLite",
                "Hybrid FTS5 + sqlite-vec local database",
                "Zero external services required for local search",
            ),
            (
                "Expose compact MCP surface",
                "Five progressive-disclosure tools over stdio and HTTP",
                "Includes search, get, timeline, save, and update",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let save = handle_rpc(
                json!({
                    "jsonrpc": "2.0",
                    "id": idx + 2,
                    "method": "tools/call",
                    "params": {
                        "name": "memory_save",
                        "arguments": {
                            "title": title,
                            "summary": summary,
                            "content": content,
                            "type": "architecture",
                            "scope": "global",
                            "files": ["src/mcp.rs"],
                            "tags": ["mcp", "rust"],
                            "client": "claude-code"
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
            let payload = tool_payload(&save);
            assert_eq!(payload["saved"], json!(true));
            assert_eq!(payload["memory"]["title"], json!(title));
            saved_ids.push(payload["memory"]["id"].as_str().unwrap().to_string());
        }

        // 2. memory_search
        let search = handle_rpc(
            json!({
                "jsonrpc": "2.0",
                "id": 10,
                "method": "tools/call",
                "params": {
                    "name": "memory_search",
                    "arguments": { "query": "Axum daemon" }
                }
            }),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        let search_payload = tool_payload(&search);
        assert_eq!(search_payload["count"], json!(1));
        assert_eq!(search_payload["results"][0]["id"], json!(saved_ids[0]));

        // 3. memory_get
        let get = handle_rpc(
            json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "tools/call",
                "params": {
                    "name": "memory_get",
                    "arguments": { "ids": [saved_ids[0], saved_ids[2]] }
                }
            }),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        let get_payload = tool_payload(&get);
        assert_eq!(get_payload["count"], json!(2));
        let fetched = get_payload["memories"].as_array().unwrap();
        assert!(fetched.iter().any(|m| {
            m["id"] == json!(saved_ids[0])
                && m["content"] == json!("Fast ~3ms hooks and 20MB RAM")
                && m["files"] == json!(["src/mcp.rs"])
                && m["tags"] == json!(["mcp", "rust"])
        }));

        // 4. memory_timeline
        let timeline = handle_rpc(
            json!({
                "jsonrpc": "2.0",
                "id": 12,
                "method": "tools/call",
                "params": {
                    "name": "memory_timeline",
                    "arguments": {
                        "memory_id": saved_ids[1],
                        "before": 2,
                        "after": 2
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
        let timeline_payload = tool_payload(&timeline);
        assert_eq!(timeline_payload["count"], json!(3));
        let timeline_ids: Vec<&str> = timeline_payload["timeline"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["id"].as_str())
            .collect();
        assert!(timeline_ids.contains(&saved_ids[0].as_str()));
        assert!(timeline_ids.contains(&saved_ids[1].as_str()));
        assert!(timeline_ids.contains(&saved_ids[2].as_str()));

        // 5. memory_update (edit + pin, verify idempotency and missing-ID handling)
        let update_args = json!({
            "id": saved_ids[0],
            "title": "Use Rust Axum local daemon",
            "status": "pinned"
        });
        let update = handle_rpc(
            json!({
                "jsonrpc": "2.0",
                "id": 13,
                "method": "tools/call",
                "params": { "name": "memory_update", "arguments": update_args }
            }),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        let update_payload = tool_payload(&update);
        assert_eq!(update_payload["updated"], json!(true));
        assert_eq!(
            update_payload["memory"]["title"],
            json!("Use Rust Axum local daemon")
        );
        assert_eq!(update_payload["memory"]["status"], json!("pinned"));

        let update_repeat = handle_rpc(
            json!({
                "jsonrpc": "2.0",
                "id": 14,
                "method": "tools/call",
                "params": { "name": "memory_update", "arguments": update_args }
            }),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        let repeat_payload = tool_payload(&update_repeat);
        assert_eq!(repeat_payload["updated"], json!(true));
        assert_eq!(
            repeat_payload["memory"]["title"],
            update_payload["memory"]["title"]
        );
        assert_eq!(
            repeat_payload["memory"]["status"],
            update_payload["memory"]["status"]
        );

        let missing = handle_rpc(
            json!({
                "jsonrpc": "2.0",
                "id": 15,
                "method": "tools/call",
                "params": {
                    "name": "memory_update",
                    "arguments": { "id": "mem_nonexistent", "status": "archived" }
                }
            }),
            &db,
            &embedder,
            &tx,
            None,
        )
        .await
        .unwrap();
        let missing_payload = tool_payload(&missing);
        assert_eq!(missing_payload["updated"], json!(false));
    }
}
