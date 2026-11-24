use crate::db::MemoryDb;
use crate::distill::DistillHandle;
use crate::project::resolve_project;
use crate::scrub::scrub_and_truncate;
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn default_t3_db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".t3/userdata/statev2.sqlite"))
}

/// Spawns a read-only background watcher on `~/.t3/userdata/statev2.sqlite` (SQLite WAL).
/// Captures Antigravity-in-T3 (where T3 isolates global GEMINI_HOME hooks/MCP) and deduplicates
/// Claude/Codex threads already captured via native hooks using `provider_thread_id`.
pub fn spawn_t3_watcher(db: MemoryDb, distill: DistillHandle) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(12));
        loop {
            interval.tick().await;
            let Ok(settings) = db.get_settings() else {
                continue;
            };
            if !settings.t3_watcher_enabled {
                continue;
            }
            let Some(t3_path) = default_t3_db_path() else {
                continue;
            };
            if !t3_path.exists() {
                continue;
            }
            let db_clone = db.clone();
            let distill_clone = distill.clone();
            let _ = tokio::task::spawn_blocking(move || {
                poll_t3_once(&t3_path, &db_clone, &distill_clone);
            })
            .await;
        }
    });
}

fn poll_t3_once(t3_path: &Path, db: &MemoryDb, distill: &DistillHandle) {
    let watermark = match db.get_meta("t3_watermark") {
        Ok(Some(ts)) => ts,
        Ok(None) => {
            // Initialize watermark to "now" on first run so we don't ingest months of historical threads.
            let now = Utc::now().to_rfc3339();
            let _ = db.set_meta("t3_watermark", &now);
            return;
        }
        Err(_) => return,
    };

    let Ok(conn) = Connection::open_with_flags(
        t3_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return;
    };
    let _ = conn.busy_timeout(Duration::from_millis(2000));

    let sql = "
        SELECT
            m.message_id,
            m.thread_id,
            m.role,
            COALESCE(m.search_text, ''),
            m.updated_at,
            COALESCE(t.default_provider, 't3-code'),
            COALESCE(p.workspace_root, ''),
            COALESCE(pt.provider_session_id, ''),
            COALESCE(pt.provider_thread_id, '')
        FROM orchestration_v2_projection_messages m
        JOIN orchestration_v2_projection_threads t ON t.thread_id = m.thread_id
        LEFT JOIN projection_projects p ON p.project_id = t.project_id
        LEFT JOIN orchestration_v2_projection_provider_threads pt
            ON pt.thread_id = m.thread_id AND pt.provider_thread_id = t.active_provider_thread_id
        WHERE m.streaming = 0 AND m.updated_at > ?1
        ORDER BY m.updated_at ASC
        LIMIT 30
    ";

    let Ok(mut stmt) = conn.prepare(sql) else {
        return;
    };

    let rows = match stmt.query_map([&watermark], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
        ))
    }) {
        Ok(rows) => rows.filter_map(Result::ok).collect::<Vec<_>>(),
        Err(_) => return,
    };

    let mut newest_ts = watermark;
    for (
        _msg_id,
        thread_id,
        role,
        text,
        updated_at,
        provider,
        workspace_root,
        provider_session_id,
        provider_thread_id,
    ) in rows
    {
        newest_ts = updated_at;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Deduplicate against native Claude Code / Codex hooks checking BOTH provider IDs
        let already_captured = (!provider_session_id.is_empty()
            && db
                .has_external_session(&provider_session_id)
                .unwrap_or(false))
            || (!provider_thread_id.is_empty()
                && db
                    .has_external_session(&provider_thread_id)
                    .unwrap_or(false));
        if already_captured {
            continue;
        }
        // Skip Claude/Codex in T3 when they already pass through native hooks; prioritize Antigravity & other isolated providers
        let source_client = if provider.to_ascii_lowercase().contains("antigravity") {
            "antigravity"
        } else {
            "t3-code"
        };

        let cwd = if workspace_root.is_empty() {
            Path::new(".")
        } else {
            Path::new(&workspace_root)
        };
        let proj_ident = resolve_project(cwd);
        let Ok(project) = db.upsert_project(&proj_ident) else {
            continue;
        };
        if project.privacy_mode == "paused" {
            continue;
        }
        let ext_id = format!("t3:{thread_id}");
        let Ok(session) = db.upsert_session(
            &project.id,
            source_client,
            &ext_id,
            proj_ident.branch.as_deref(),
        ) else {
            continue;
        };

        let clean = json!({
            "role": role,
            "provider": provider,
            "text": scrub_and_truncate(trimmed, 2_000),
        });
        let ev_type = if role == "user" {
            "prompt"
        } else {
            "t3_message"
        };
        let _ = db.insert_raw_event(
            &session.id,
            &project.id,
            source_client,
            ev_type,
            None,
            &clean.to_string(),
        );
        let _ = db.touch_client_activity("t3-code", true);

        if role == "assistant" {
            distill.queue_session(session.id);
        }
    }

    let _ = db.set_meta("t3_watermark", &newest_ts);
}
