use crate::config::{EMBEDDING_DIM, Settings, ensure_parent_dir};
use crate::models::*;
use crate::project::ProjectIdentity;
use crate::scrub::{redact_secrets, scrub_and_truncate};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, Once};
use uuid::Uuid;

static SQLITE_VEC_INIT: Once = Once::new();

pub fn register_sqlite_vec() {
    SQLITE_VEC_INIT.call_once(|| unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

pub type ClientHeartbeatMap = HashMap<String, (Option<String>, Option<String>, Option<String>)>;

#[derive(Clone)]
pub struct MemoryDb {
    conn: Arc<Mutex<Connection>>,
}

impl MemoryDb {
    pub fn open(path: &Path) -> Result<Self> {
        register_sqlite_vec();
        ensure_parent_dir(path)?;
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open SQLite DB at {}", path.display()))?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(&format!(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA busy_timeout = 5000;

            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                remote_url TEXT,
                root_paths_json TEXT NOT NULL DEFAULT '[]',
                privacy_mode TEXT NOT NULL DEFAULT 'full',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                source_client TEXT NOT NULL,
                external_session_id TEXT NOT NULL,
                branch TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                last_turn_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                UNIQUE(source_client, external_session_id)
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id, last_turn_at DESC);

            CREATE TABLE IF NOT EXISTS raw_events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                project_id TEXT NOT NULL,
                source_client TEXT NOT NULL,
                event_type TEXT NOT NULL,
                tool_name TEXT,
                payload_json TEXT NOT NULL,
                distilled INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_raw_events_pending ON raw_events(distilled, session_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_raw_events_created ON raw_events(created_at);

            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                scope TEXT NOT NULL DEFAULT 'project',
                project_id TEXT,
                session_id TEXT,
                source_client TEXT NOT NULL DEFAULT 'mcp',
                memory_type TEXT NOT NULL DEFAULT 'pattern',
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                content TEXT NOT NULL,
                files_json TEXT NOT NULL DEFAULT '[]',
                tags_json TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL DEFAULT 'active',
                superseded_by TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memories_project ON memories(project_id, status, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope, status, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memories_client ON memories(source_client, updated_at DESC);

            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                title,
                summary,
                content,
                tags_json,
                files_json,
                content='memories',
                content_rowid='rowid',
                tokenize='unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, title, summary, content, tags_json, files_json)
                VALUES (new.rowid, new.title, new.summary, new.content, new.tags_json, new.files_json);
            END;
            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, title, summary, content, tags_json, files_json)
                VALUES ('delete', old.rowid, old.title, old.summary, old.content, old.tags_json, old.files_json);
            END;
            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, title, summary, content, tags_json, files_json)
                VALUES ('delete', old.rowid, old.title, old.summary, old.content, old.tags_json, old.files_json);
                INSERT INTO memories_fts(rowid, title, summary, content, tags_json, files_json)
                VALUES (new.rowid, new.title, new.summary, new.content, new.tags_json, new.files_json);
            END;

            CREATE VIRTUAL TABLE IF NOT EXISTS memories_vec USING vec0(
                memory_rowid INTEGER PRIMARY KEY,
                embedding float[{EMBEDDING_DIM}] distance_metric=cosine
            );

            CREATE TABLE IF NOT EXISTS usage_ledger (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                operation TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
                session_id TEXT,
                status TEXT NOT NULL DEFAULT 'ok',
                error_message TEXT,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_usage_created ON usage_ledger(created_at DESC);

            CREATE TABLE IF NOT EXISTS client_connections (
                client_id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                detected INTEGER NOT NULL DEFAULT 0,
                connected INTEGER NOT NULL DEFAULT 0,
                needs_manual_step INTEGER NOT NULL DEFAULT 0,
                manual_step_hint TEXT,
                config_paths_json TEXT NOT NULL DEFAULT '[]',
                config_backup_path TEXT,
                last_heartbeat_at TEXT,
                last_event_at TEXT,
                status_note TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "
        ))?;
        Ok(())
    }

    // --- Settings ---

    pub fn get_settings(&self) -> Result<Settings> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let mut map = HashMap::new();
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(Settings::from_map(&map))
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        for (k, v) in settings.to_map() {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Auto-skip Onboarding once for users upgrading from versions before `onboarding_completed` existed.
    pub fn seed_onboarding_if_existing(&self, has_configured_clients: bool) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let existing: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'onboarding_completed'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(val) = existing {
            return Ok(val == "true" || val == "1");
        }
        let has_memories: bool =
            conn.query_row("SELECT EXISTS(SELECT 1 FROM memories LIMIT 1)", [], |r| {
                r.get::<_, i64>(0)
            })? != 0;
        // Ignore passive t3-code (marked connected by default whenever T3 Code is installed).
        let has_prior_conn: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM client_connections WHERE connected = 1 AND client_id != 't3-code' LIMIT 1)",
            [],
            |r| r.get::<_, i64>(0),
        )? != 0;
        if has_memories || has_prior_conn || has_configured_clients {
            conn.execute(
                "INSERT INTO settings(key, value) VALUES ('onboarding_completed', 'true')
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [],
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![format!("_meta:{key}")],
            |r| r.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("_meta:{key}"), value],
        )?;
        Ok(())
    }

    // --- Projects & Sessions ---

    pub fn upsert_project(&self, ident: &ProjectIdentity) -> Result<Project> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        let existing: Option<(String, String, String)> = conn
            .query_row(
                "SELECT root_paths_json, privacy_mode, created_at FROM projects WHERE id = ?1",
                params![ident.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        if let Some((paths_json, privacy_mode, created_at)) = existing {
            let mut paths: Vec<String> = serde_json::from_str(&paths_json).unwrap_or_default();
            if !paths.iter().any(|p| p == &ident.root_path) {
                paths.push(ident.root_path.clone());
            }
            let updated_json = serde_json::to_string(&paths)?;
            conn.execute(
                "UPDATE projects SET name = ?2, remote_url = COALESCE(?3, remote_url),
                 root_paths_json = ?4, updated_at = ?5 WHERE id = ?1",
                params![ident.id, ident.name, ident.remote_url, updated_json, now],
            )?;
            Ok(Project {
                id: ident.id.clone(),
                name: ident.name.clone(),
                remote_url: ident.remote_url.clone(),
                root_paths: paths,
                privacy_mode,
                created_at,
                updated_at: now,
            })
        } else {
            let paths = vec![ident.root_path.clone()];
            let paths_json = serde_json::to_string(&paths)?;
            conn.execute(
                "INSERT INTO projects(id, name, remote_url, root_paths_json, privacy_mode, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'full', ?5, ?5)",
                params![ident.id, ident.name, ident.remote_url, paths_json, now],
            )?;
            Ok(Project {
                id: ident.id.clone(),
                name: ident.name.clone(),
                remote_url: ident.remote_url.clone(),
                root_paths: paths,
                privacy_mode: "full".into(),
                created_at: now.clone(),
                updated_at: now,
            })
        }
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, remote_url, root_paths_json, privacy_mode, created_at, updated_at
             FROM projects ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let paths_json: String = r.get(3)?;
            Ok(Project {
                id: r.get(0)?,
                name: r.get(1)?,
                remote_url: r.get(2)?,
                root_paths: serde_json::from_str(&paths_json).unwrap_or_default(),
                privacy_mode: r.get(4)?,
                created_at: r.get(5)?,
                updated_at: r.get(6)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, remote_url, root_paths_json, privacy_mode, created_at, updated_at
             FROM projects WHERE id = ?1",
            params![id],
            |r| {
                let paths_json: String = r.get(3)?;
                Ok(Project {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    remote_url: r.get(2)?,
                    root_paths: serde_json::from_str(&paths_json).unwrap_or_default(),
                    privacy_mode: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_project_privacy(&self, project_id: &str, mode: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let normalized = normalize_privacy_mode(mode);
        let now = Utc::now().to_rfc3339();
        let rows = conn.execute(
            "UPDATE projects SET privacy_mode = ?2, updated_at = ?3 WHERE id = ?1",
            params![project_id, normalized, now],
        )?;
        Ok(rows > 0)
    }

    pub fn is_project_local_only(&self, project_id: Option<&str>) -> bool {
        project_id
            .and_then(|pid| self.get_project(pid).ok().flatten())
            .map(|p| p.privacy_mode == "local_only")
            .unwrap_or(false)
    }

    pub fn upsert_session(
        &self,
        project_id: &str,
        source_client: &str,
        external_session_id: &str,
        branch: Option<&str>,
    ) -> Result<Session> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        let client = normalize_client(Some(source_client));

        let existing: Option<Session> = conn
            .query_row(
                "SELECT id, project_id, source_client, external_session_id, branch, started_at, ended_at, last_turn_at, status
                 FROM sessions WHERE source_client = ?1 AND external_session_id = ?2",
                params![client, external_session_id],
                row_to_session,
            )
            .optional()?;

        if let Some(mut s) = existing {
            conn.execute(
                "UPDATE sessions SET last_turn_at = ?2, branch = COALESCE(?3, branch), status = 'active' WHERE id = ?1",
                params![s.id, now, branch],
            )?;
            s.last_turn_at = now;
            if let Some(b) = branch {
                s.branch = Some(b.to_string());
            }
            s.status = "active".into();
            return Ok(s);
        }

        let id = format!("sess_{}", Uuid::now_v7().simple());
        conn.execute(
            "INSERT INTO sessions(id, project_id, source_client, external_session_id, branch, started_at, last_turn_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 'active')",
            params![id, project_id, client, external_session_id, branch, now],
        )?;
        Ok(Session {
            id,
            project_id: project_id.to_string(),
            source_client: client,
            external_session_id: external_session_id.to_string(),
            branch: branch.map(ToOwned::to_owned),
            started_at: now.clone(),
            ended_at: None,
            last_turn_at: now,
            status: "active".into(),
        })
    }

    pub fn has_external_session(&self, external_session_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM sessions WHERE external_session_id = ?1 LIMIT 1",
                params![external_session_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn end_session(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE sessions SET status = 'ended', ended_at = ?2 WHERE id = ?1",
            params![session_id, now],
        )?;
        Ok(())
    }

    // --- Raw Events (0-token capture) ---

    pub fn insert_raw_event(
        &self,
        session_id: &str,
        project_id: &str,
        source_client: &str,
        event_type: &str,
        tool_name: Option<&str>,
        payload_json: &str,
    ) -> Result<RawEvent> {
        let conn = self.conn.lock().unwrap();
        let id = format!("evt_{}", Uuid::now_v7().simple());
        let now = Utc::now().to_rfc3339();
        let client = normalize_client(Some(source_client));
        let clean_payload = scrub_and_truncate(payload_json, 6_000);
        conn.execute(
            "INSERT INTO raw_events(id, session_id, project_id, source_client, event_type, tool_name, payload_json, distilled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
            params![id, session_id, project_id, client, event_type, tool_name, clean_payload, now],
        )?;
        conn.execute(
            "UPDATE sessions SET last_turn_at = ?2 WHERE id = ?1",
            params![session_id, now],
        )?;
        Ok(RawEvent {
            id,
            session_id: session_id.to_string(),
            project_id: project_id.to_string(),
            source_client: client,
            event_type: event_type.to_string(),
            tool_name: tool_name.map(ToOwned::to_owned),
            payload_json: clean_payload,
            distilled: false,
            created_at: now,
        })
    }

    pub fn pending_events_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<RawEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, project_id, source_client, event_type, tool_name, payload_json, distilled, created_at
             FROM raw_events WHERE session_id = ?1 AND distilled = 0
             ORDER BY created_at ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], row_to_raw_event)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn sessions_with_idle_pending_events(&self, idle_seconds: i64) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let cutoff = (Utc::now() - chrono::Duration::seconds(idle_seconds)).to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT r.session_id
             FROM raw_events r
             WHERE r.distilled = 0
             GROUP BY r.session_id
             HAVING MAX(r.created_at) <= ?1
             LIMIT 10",
        )?;
        let rows = stmt.query_map(params![cutoff], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn mark_events_distilled(&self, event_ids: &[String]) -> Result<()> {
        if event_ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        for id in event_ids {
            tx.execute(
                "UPDATE raw_events SET distilled = 1 WHERE id = ?1",
                params![id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn prune_old_raw_events(&self, retention_days: i64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let cutoff_distilled =
            (Utc::now() - chrono::Duration::days(retention_days.max(1))).to_rfc3339();
        let cutoff_stale =
            (Utc::now() - chrono::Duration::days((retention_days * 2).max(2))).to_rfc3339();
        let n1 = conn.execute(
            "DELETE FROM raw_events WHERE distilled = 1 AND created_at < ?1",
            params![cutoff_distilled],
        )?;
        let n2 = conn.execute(
            "DELETE FROM raw_events WHERE created_at < ?1",
            params![cutoff_stale],
        )?;
        Ok(n1 + n2)
    }

    // --- Memories CRUD + Embeddings ---

    pub fn create_memory(
        &self,
        input: NewMemoryInput,
        embedding: Option<&[f32]>,
    ) -> Result<Memory> {
        let conn = self.conn.lock().unwrap();
        let id = format!("mem_{}", Uuid::now_v7().simple());
        let now = Utc::now().to_rfc3339();
        let scope =
            normalize_scope(input.scope.as_deref(), input.project_id.as_deref()).to_string();
        let project_id = if scope == "global" {
            None
        } else {
            input.project_id.clone()
        };
        let source_client = normalize_client(input.source_client.as_deref());
        let memory_type = normalize_memory_type(&input.memory_type).to_string();
        let status = normalize_status(input.status.as_deref()).to_string();
        let title = scrub_and_truncate(&input.title, 200);
        let summary = scrub_and_truncate(&input.summary, 500);
        let content = scrub_and_truncate(&input.content, 16_000);
        let files: Vec<String> = input
            .files
            .unwrap_or_default()
            .into_iter()
            .map(|f| redact_secrets(&f))
            .take(50)
            .collect();
        let tags: Vec<String> = input
            .tags
            .unwrap_or_default()
            .into_iter()
            .map(|t| redact_secrets(&t).to_ascii_lowercase())
            .take(30)
            .collect();
        let files_json = serde_json::to_string(&files)?;
        let tags_json = serde_json::to_string(&tags)?;

        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO memories(id, scope, project_id, session_id, source_client, memory_type, title, summary, content, files_json, tags_json, status, superseded_by, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13, ?13)",
            params![
                id,
                scope,
                project_id,
                input.session_id,
                source_client,
                memory_type,
                title,
                summary,
                content,
                files_json,
                tags_json,
                status,
                now
            ],
        )?;
        let rowid = tx.last_insert_rowid();

        if let Some(vec) = embedding
            && vec.len() == EMBEDDING_DIM
        {
            let bytes = vec_to_le_bytes(vec);
            tx.execute(
                "INSERT OR REPLACE INTO memories_vec(memory_rowid, embedding) VALUES (?1, ?2)",
                params![rowid, bytes],
            )?;
        }

        if let Some(old_id) = input.supersedes_id.as_deref()
            && !old_id.trim().is_empty()
        {
            tx.execute(
                    "UPDATE memories SET status = 'superseded', superseded_by = ?2, updated_at = ?3 WHERE id = ?1",
                    params![old_id, id, now],
                )?;
        }

        tx.commit()?;

        let project_name = project_id.as_deref().and_then(|pid| {
            conn.query_row(
                "SELECT name FROM projects WHERE id = ?1",
                params![pid],
                |r| r.get::<_, String>(0),
            )
            .ok()
        });

        Ok(Memory {
            id,
            scope,
            project_id,
            project_name,
            session_id: input.session_id,
            source_client,
            memory_type,
            title,
            summary,
            content,
            files,
            tags,
            status,
            superseded_by: None,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn set_memory_embedding(&self, memory_id: &str, embedding: &[f32]) -> Result<()> {
        if embedding.len() != EMBEDDING_DIM {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        if let Some(rowid) = conn
            .query_row(
                "SELECT rowid FROM memories WHERE id = ?1",
                params![memory_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            let bytes = vec_to_le_bytes(embedding);
            conn.execute(
                "INSERT OR REPLACE INTO memories_vec(memory_rowid, embedding) VALUES (?1, ?2)",
                params![rowid, bytes],
            )?;
        }
        Ok(())
    }

    pub fn get_memories_by_ids(&self, ids: &[String]) -> Result<Vec<Memory>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::with_capacity(ids.len());
        let mut stmt = conn.prepare(
            "SELECT m.id, m.scope, m.project_id, p.name, m.session_id, m.source_client,
                    m.memory_type, m.title, m.summary, m.content, m.files_json, m.tags_json,
                    m.status, m.superseded_by, m.created_at, m.updated_at
             FROM memories m
             LEFT JOIN projects p ON p.id = m.project_id
             WHERE m.id = ?1",
        )?;
        for id in ids {
            if let Some(mem) = stmt.query_row(params![id], row_to_memory).optional()? {
                out.push(mem);
            }
        }
        Ok(out)
    }

    pub fn update_memory(
        &self,
        id: &str,
        patch: UpdateMemoryInput,
        embedding: Option<&[f32]>,
    ) -> Result<Option<Memory>> {
        let existing = self.get_memories_by_ids(&[id.to_string()])?.pop();
        let Some(mut mem) = existing else {
            return Ok(None);
        };
        if let Some(st) = patch.status {
            mem.status = normalize_status(Some(&st)).to_string();
        }
        if let Some(t) = patch.title {
            mem.title = scrub_and_truncate(&t, 200);
        }
        if let Some(s) = patch.summary {
            mem.summary = scrub_and_truncate(&s, 500);
        }
        if let Some(c) = patch.content {
            mem.content = scrub_and_truncate(&c, 16_000);
        }
        if let Some(f) = patch.files {
            mem.files = f.into_iter().map(|x| redact_secrets(&x)).take(50).collect();
        }
        if let Some(t) = patch.tags {
            mem.tags = t
                .into_iter()
                .map(|x| redact_secrets(&x).to_ascii_lowercase())
                .take(30)
                .collect();
        }
        if let Some(sup) = patch.superseded_by {
            mem.superseded_by = if sup.trim().is_empty() {
                None
            } else {
                mem.status = "superseded".into();
                Some(sup)
            };
        }
        mem.updated_at = Utc::now().to_rfc3339();

        let files_json = serde_json::to_string(&mem.files)?;
        let tags_json = serde_json::to_string(&mem.tags)?;
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE memories
                 SET title = ?2, summary = ?3, content = ?4, files_json = ?5, tags_json = ?6,
                     status = ?7, superseded_by = ?8, updated_at = ?9
                 WHERE id = ?1",
                params![
                    mem.id,
                    mem.title,
                    mem.summary,
                    mem.content,
                    files_json,
                    tags_json,
                    mem.status,
                    mem.superseded_by,
                    mem.updated_at
                ],
            )?;
        }
        if let Some(vec) = embedding {
            self.set_memory_embedding(&mem.id, vec)?;
        }
        Ok(Some(mem))
    }

    pub fn delete_memory(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        if let Some(rowid) = tx
            .query_row(
                "SELECT rowid FROM memories WHERE id = ?1",
                params![id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            let _ = tx.execute(
                "DELETE FROM memories_vec WHERE memory_rowid = ?1",
                params![rowid],
            );
            tx.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
            tx.commit()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Hybrid FTS5 (BM25) + sqlite-vec (cosine) search fused with Reciprocal Rank Fusion (RRF).
    pub fn search_memories(
        &self,
        filter: &SearchFilter,
        query_embedding: Option<&[f32]>,
    ) -> Result<Vec<MemoryHit>> {
        let limit = filter.limit.unwrap_or(20).clamp(1, 100);
        let include_inactive = filter.include_inactive.unwrap_or(false);
        let query = filter.query.as_deref().unwrap_or("").trim();

        let conn = self.conn.lock().unwrap();
        if query.is_empty() {
            return list_recent_memories_locked(&conn, filter, include_inactive, limit);
        }

        let mut rrf_scores: HashMap<i64, f64> = HashMap::new();

        // 1. FTS5 BM25 pass
        if let Some(fts_query) = sanitize_fts_query(query) {
            let mut stmt = conn.prepare(
                "SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?1 ORDER BY bm25(memories_fts) LIMIT 60",
            )?;
            let rows = stmt
                .query_map(params![fts_query], |r| r.get::<_, i64>(0))?
                .filter_map(Result::ok);
            for (rank, rowid) in rows.enumerate() {
                *rrf_scores.entry(rowid).or_insert(0.0) += 1.0 / (60.0 + (rank as f64) + 1.0);
            }
        }

        // 2. sqlite-vec KNN pass (when embedding available)
        if let Some(vec) = query_embedding
            && vec.len() == EMBEDDING_DIM
        {
            let bytes = vec_to_le_bytes(vec);
            let mut stmt = conn.prepare(
                "SELECT memory_rowid, distance FROM memories_vec
                     WHERE embedding MATCH ?1 AND k = 60
                     ORDER BY distance ASC",
            )?;
            if let Ok(rows) = stmt.query_map(params![bytes], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
            }) {
                for (rank, item) in rows.filter_map(Result::ok).enumerate() {
                    let (rowid, distance) = item;
                    if distance < 0.85 {
                        *rrf_scores.entry(rowid).or_insert(0.0) +=
                            1.0 / (60.0 + (rank as f64) + 1.0);
                    }
                }
            }
        }

        // 3. Substring LIKE fallback if FTS5 & vector found nothing (e.g. exact path/symbol substring)
        if rrf_scores.is_empty() {
            let like = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
            let mut stmt = conn.prepare(
                "SELECT rowid FROM memories
                 WHERE title LIKE ?1 ESCAPE '\\' OR summary LIKE ?1 ESCAPE '\\' OR content LIKE ?1 ESCAPE '\\' OR files_json LIKE ?1 ESCAPE '\\'
                 ORDER BY updated_at DESC LIMIT 40",
            )?;
            let rows = stmt
                .query_map(params![like], |r| r.get::<_, i64>(0))?
                .filter_map(Result::ok);
            for (rank, rowid) in rows.enumerate() {
                *rrf_scores.entry(rowid).or_insert(0.0) += 0.5 / (60.0 + (rank as f64) + 1.0);
            }
        }

        let mut hits = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT m.rowid, m.id, m.scope, m.project_id, p.name, m.source_client,
                    m.memory_type, m.title, m.summary, m.status, m.files_json, m.tags_json, m.updated_at
             FROM memories m
             LEFT JOIN projects p ON p.id = m.project_id
             WHERE m.rowid = ?1",
        )?;

        for (rowid, base_score) in rrf_scores {
            let Some(mut hit) = stmt
                .query_row(params![rowid], |r| {
                    let files_json: String = r.get(10)?;
                    let tags_json: String = r.get(11)?;
                    Ok(MemoryHit {
                        id: r.get(1)?,
                        scope: r.get(2)?,
                        project_id: r.get(3)?,
                        project_name: r.get(4)?,
                        source_client: r.get(5)?,
                        memory_type: r.get(6)?,
                        title: r.get(7)?,
                        summary: r.get(8)?,
                        status: r.get(9)?,
                        files: serde_json::from_str(&files_json).unwrap_or_default(),
                        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                        updated_at: r.get(12)?,
                        score: base_score,
                    })
                })
                .optional()?
            else {
                continue;
            };

            if !matches_filter(&hit, filter, include_inactive) {
                continue;
            }
            if hit.status == "pinned" {
                hit.score += 0.015;
            }
            if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&hit.updated_at) {
                let age_days =
                    (Utc::now() - ts.with_timezone(&Utc)).num_hours().max(0) as f64 / 24.0;
                hit.score += 0.005 * (-age_days / 30.0).exp();
            }
            hits.push(hit);
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn memory_timeline(
        &self,
        anchor_id: &str,
        before: usize,
        after: usize,
    ) -> Result<Vec<MemoryHit>> {
        let before = before.clamp(0, 20);
        let after = after.clamp(0, 20);
        let conn = self.conn.lock().unwrap();

        let anchor: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT created_at, project_id FROM memories WHERE id = ?1",
                params![anchor_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;

        let Some((anchor_time, project_id)) = anchor else {
            return Ok(Vec::new());
        };

        let mut older =
            query_timeline_side(&conn, project_id.as_deref(), &anchor_time, true, before)?;
        older.reverse();
        // Query anchor directly on the already-locked connection (never re-lock self.conn!).
        let anchor_mem = conn
            .query_row(
                "SELECT m.id, m.scope, m.project_id, p.name, m.session_id, m.source_client,
                        m.memory_type, m.title, m.summary, m.content, m.files_json, m.tags_json,
                        m.status, m.superseded_by, m.created_at, m.updated_at
                 FROM memories m
                 LEFT JOIN projects p ON p.id = m.project_id
                 WHERE m.id = ?1",
                params![anchor_id],
                row_to_memory,
            )
            .optional()?;
        let newer = query_timeline_side(&conn, project_id.as_deref(), &anchor_time, false, after)?;

        let mut out = older;
        if let Some(m) = anchor_mem {
            out.push(MemoryHit {
                id: m.id,
                scope: m.scope,
                project_id: m.project_id,
                project_name: m.project_name,
                source_client: m.source_client,
                memory_type: m.memory_type,
                title: m.title,
                summary: m.summary,
                status: m.status,
                files: m.files,
                tags: m.tags,
                updated_at: m.updated_at,
                score: 1.0,
            });
        }
        out.extend(newer);
        Ok(out)
    }

    /// Builds an ultra-lean (~300-500 token) SessionStart briefing shared across all clients.
    pub fn build_session_briefing(
        &self,
        project_id: Option<&str>,
        token_budget: usize,
    ) -> Result<(String, usize)> {
        let max_chars = token_budget.clamp(100, 2_000) * 4;
        let conn = self.conn.lock().unwrap();
        let mut seen = HashSet::new();
        let mut pinned_lines = Vec::new();
        let mut global_lines = Vec::new();
        let mut project_lines = Vec::new();

        // 1. Pinned memories (project + global)
        let mut stmt = conn.prepare(
            "SELECT id, source_client, memory_type, title, summary
             FROM memories
             WHERE status = 'pinned' AND (scope = 'global' OR project_id = ?1)
             ORDER BY updated_at DESC LIMIT 5",
        )?;
        for row in stmt
            .query_map(params![project_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .filter_map(Result::ok)
        {
            if seen.insert(row.0.clone()) {
                pinned_lines.push(format!(
                    "- [pinned|{}|{}] {} — {} (id:{})",
                    row.1, row.2, row.3, row.4, row.0
                ));
            }
        }

        // 2. Global user preferences
        let mut stmt = conn.prepare(
            "SELECT id, source_client, memory_type, title, summary
             FROM memories
             WHERE scope = 'global' AND status = 'active'
             ORDER BY updated_at DESC LIMIT 4",
        )?;
        for row in stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .filter_map(Result::ok)
        {
            if seen.insert(row.0.clone()) {
                global_lines.push(format!("- [{}|{}] {} (id:{})", row.1, row.2, row.3, row.0));
            }
        }

        // 3. Recent cross-client active project memories
        if let Some(pid) = project_id {
            let mut stmt = conn.prepare(
                "SELECT id, source_client, memory_type, title, summary
                 FROM memories
                 WHERE project_id = ?1 AND status = 'active'
                 ORDER BY updated_at DESC LIMIT 8",
            )?;
            for row in stmt
                .query_map(params![pid], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?
                .filter_map(Result::ok)
            {
                if seen.insert(row.0.clone()) {
                    project_lines.push(format!(
                        "- [{}|{}] {}: {} (id:{})",
                        row.1, row.2, row.3, row.4, row.0
                    ));
                }
            }
        }

        let count = pinned_lines.len() + global_lines.len() + project_lines.len();
        if count == 0 {
            return Ok((String::new(), 0));
        }

        let mut buf = String::from(
            "# 0x1-ai-memory (Cross-Agent Briefing)\nUse `memory_get([id])` for full details or `memory_search(query)` for older context.\n",
        );
        if !pinned_lines.is_empty() {
            buf.push_str("\n## Pinned\n");
            for line in pinned_lines {
                if buf.len() + line.len() + 2 > max_chars {
                    break;
                }
                buf.push_str(&line);
                buf.push('\n');
            }
        }
        if !project_lines.is_empty() {
            buf.push_str("\n## Recent Project Memory (All AI Clients)\n");
            for line in project_lines {
                if buf.len() + line.len() + 2 > max_chars {
                    break;
                }
                buf.push_str(&line);
                buf.push('\n');
            }
        }
        if !global_lines.is_empty() {
            buf.push_str("\n## Global Preferences\n");
            for line in global_lines {
                if buf.len() + line.len() + 2 > max_chars {
                    break;
                }
                buf.push_str(&line);
                buf.push('\n');
            }
        }

        Ok((buf.trim().to_string(), count))
    }

    pub fn memories_missing_embeddings(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, Option<String>, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.project_id, m.title || '\n' || m.summary || '\n' || m.content
             FROM memories m
             LEFT JOIN memories_vec v ON v.memory_rowid = m.rowid
             WHERE v.memory_rowid IS NULL AND m.status IN ('active', 'pinned')
             ORDER BY m.updated_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    // --- Usage Ledger & Budget ---

    #[allow(clippy::too_many_arguments)]
    pub fn record_usage(
        &self,
        provider: &str,
        model: &str,
        operation: &str,
        input_tokens: i64,
        output_tokens: i64,
        estimated_cost_usd: f64,
        session_id: Option<&str>,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<UsageEntry> {
        let conn = self.conn.lock().unwrap();
        let id = format!("usg_{}", Uuid::now_v7().simple());
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO usage_ledger(id, provider, model, operation, input_tokens, output_tokens, estimated_cost_usd, session_id, status, error_message, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                provider,
                model,
                operation,
                input_tokens,
                output_tokens,
                estimated_cost_usd,
                session_id,
                status,
                error_message,
                now
            ],
        )?;
        Ok(UsageEntry {
            id,
            provider: provider.to_string(),
            model: model.to_string(),
            operation: operation.to_string(),
            input_tokens,
            output_tokens,
            estimated_cost_usd,
            session_id: session_id.map(ToOwned::to_owned),
            status: status.to_string(),
            error_message: error_message.map(ToOwned::to_owned),
            created_at: now,
        })
    }

    pub fn recent_usage(&self, limit: usize) -> Result<Vec<UsageEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, provider, model, operation, input_tokens, output_tokens, estimated_cost_usd, session_id, status, error_message, created_at
             FROM usage_ledger ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(UsageEntry {
                id: r.get(0)?,
                provider: r.get(1)?,
                model: r.get(2)?,
                operation: r.get(3)?,
                input_tokens: r.get(4)?,
                output_tokens: r.get(5)?,
                estimated_cost_usd: r.get(6)?,
                session_id: r.get(7)?,
                status: r.get(8)?,
                error_message: r.get(9)?,
                created_at: r.get(10)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn usage_summary(
        &self,
        settings: &Settings,
        embedding_status: String,
    ) -> Result<UsageSummary> {
        let conn = self.conn.lock().unwrap();
        let today_prefix = format!("{}%", Utc::now().format("%Y-%m-%d"));
        let (in_tok, out_tok, cost, calls, blocked): (i64, i64, f64, i64, i64) = conn.query_row(
            "SELECT
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(estimated_cost_usd), 0.0),
                COUNT(*),
                COALESCE(SUM(CASE WHEN status = 'budget_blocked' THEN 1 ELSE 0 END), 0)
             FROM usage_ledger WHERE created_at LIKE ?1",
            params![today_prefix],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        let total_memories: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE status IN ('active','pinned')",
            [],
            |r| r.get(0),
        )?;
        let total_projects: i64 =
            conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))?;
        let pending_raw_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM raw_events WHERE distilled = 0",
            [],
            |r| r.get(0),
        )?;

        let total_tokens = in_tok + out_tok;
        let budget_exhausted = (settings.daily_token_cap > 0
            && total_tokens >= settings.daily_token_cap)
            || (settings.daily_cost_cap_usd > 0.0 && cost >= settings.daily_cost_cap_usd);

        Ok(UsageSummary {
            today_input_tokens: in_tok,
            today_output_tokens: out_tok,
            today_total_tokens: total_tokens,
            today_cost_usd: cost,
            today_calls: calls,
            today_blocked: blocked,
            total_memories,
            total_projects,
            pending_raw_events,
            daily_token_cap: settings.daily_token_cap,
            daily_cost_cap_usd: settings.daily_cost_cap_usd,
            budget_exhausted,
            active_provider: settings.provider.clone(),
            active_model: settings.model.clone(),
            embedding_status,
        })
    }

    // --- Client Connections Heartbeat ---

    pub fn touch_client_activity(&self, client_id: &str, is_event: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        let norm = normalize_client(Some(client_id));
        let display = match norm.as_str() {
            "claude-code" => "Claude Code",
            "codex" => "Codex CLI & App",
            "antigravity" => "Antigravity",
            "t3-code" => "T3 Code",
            _ => &norm,
        };
        conn.execute(
            "INSERT INTO client_connections(client_id, display_name, detected, connected, last_heartbeat_at, last_event_at, updated_at)
             VALUES (?1, ?2, 1, 1, ?3, CASE WHEN ?4 THEN ?3 ELSE NULL END, ?3)
             ON CONFLICT(client_id) DO UPDATE SET
                last_heartbeat_at = excluded.last_heartbeat_at,
                last_event_at = CASE WHEN ?4 THEN excluded.last_heartbeat_at ELSE client_connections.last_event_at END,
                updated_at = excluded.updated_at",
            params![norm, display, now, is_event],
        )?;
        Ok(())
    }

    pub fn save_client_connection(&self, c: &ClientConnection) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        let paths_json = serde_json::to_string(&c.config_paths)?;
        conn.execute(
            "INSERT INTO client_connections(client_id, display_name, detected, connected, needs_manual_step, manual_step_hint, config_paths_json, config_backup_path, last_heartbeat_at, last_event_at, status_note, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(client_id) DO UPDATE SET
                display_name = excluded.display_name,
                detected = excluded.detected,
                connected = excluded.connected,
                needs_manual_step = excluded.needs_manual_step,
                manual_step_hint = excluded.manual_step_hint,
                config_paths_json = excluded.config_paths_json,
                config_backup_path = COALESCE(excluded.config_backup_path, client_connections.config_backup_path),
                last_heartbeat_at = COALESCE(client_connections.last_heartbeat_at, excluded.last_heartbeat_at),
                last_event_at = COALESCE(client_connections.last_event_at, excluded.last_event_at),
                status_note = excluded.status_note,
                updated_at = excluded.updated_at",
            params![
                c.client_id,
                c.display_name,
                c.detected as i64,
                c.connected as i64,
                c.needs_manual_step as i64,
                c.manual_step_hint,
                paths_json,
                c.config_backup_path,
                c.last_heartbeat_at,
                c.last_event_at,
                c.status_note,
                now
            ],
        )?;
        Ok(())
    }

    pub fn get_client_heartbeats(&self) -> Result<ClientHeartbeatMap> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT client_id, last_heartbeat_at, last_event_at, config_backup_path FROM client_connections",
        )?;
        let mut map = HashMap::new();
        for row in stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .filter_map(Result::ok)
        {
            map.insert(row.0, (row.1, row.2, row.3));
        }
        Ok(map)
    }
}

fn list_recent_memories_locked(
    conn: &Connection,
    filter: &SearchFilter,
    include_inactive: bool,
    limit: usize,
) -> Result<Vec<MemoryHit>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.scope, m.project_id, p.name, m.source_client,
                m.memory_type, m.title, m.summary, m.status, m.files_json, m.tags_json, m.updated_at
         FROM memories m
         LEFT JOIN projects p ON p.id = m.project_id
         ORDER BY (CASE WHEN m.status = 'pinned' THEN 0 ELSE 1 END), m.updated_at DESC
         LIMIT 250",
    )?;
    let mut hits = Vec::new();
    for row in stmt
        .query_map([], |r| {
            let files_json: String = r.get(9)?;
            let tags_json: String = r.get(10)?;
            Ok(MemoryHit {
                id: r.get(0)?,
                scope: r.get(1)?,
                project_id: r.get(2)?,
                project_name: r.get(3)?,
                source_client: r.get(4)?,
                memory_type: r.get(5)?,
                title: r.get(6)?,
                summary: r.get(7)?,
                status: r.get(8)?,
                files: serde_json::from_str(&files_json).unwrap_or_default(),
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                updated_at: r.get(11)?,
                score: 1.0,
            })
        })?
        .filter_map(Result::ok)
    {
        if matches_filter(&row, filter, include_inactive) {
            hits.push(row);
            if hits.len() >= limit {
                break;
            }
        }
    }
    Ok(hits)
}

fn matches_filter(hit: &MemoryHit, filter: &SearchFilter, include_inactive: bool) -> bool {
    if !include_inactive && hit.status != "active" && hit.status != "pinned" {
        return false;
    }
    if let Some(pid) = filter.project_id.as_deref().filter(|s| !s.is_empty())
        && hit.project_id.as_deref() != Some(pid)
        && hit.scope != "global"
    {
        return false;
    }
    if let Some(scope) = filter.scope.as_deref().filter(|s| !s.is_empty())
        && hit.scope != scope
    {
        return false;
    }
    if let Some(mtype) = filter.memory_type.as_deref().filter(|s| !s.is_empty())
        && hit.memory_type != mtype
    {
        return false;
    }
    if let Some(client) = filter.source_client.as_deref().filter(|s| !s.is_empty())
        && hit.source_client != client
    {
        return false;
    }
    true
}

fn query_timeline_side(
    conn: &Connection,
    project_id: Option<&str>,
    anchor_time: &str,
    older: bool,
    limit: usize,
) -> Result<Vec<MemoryHit>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let sql = if older {
        "SELECT m.id, m.scope, m.project_id, p.name, m.source_client,
                m.memory_type, m.title, m.summary, m.status, m.files_json, m.tags_json, m.updated_at
         FROM memories m
         LEFT JOIN projects p ON p.id = m.project_id
         WHERE (?1 IS NULL OR m.project_id = ?1) AND m.created_at < ?2
         ORDER BY m.created_at DESC LIMIT ?3"
    } else {
        "SELECT m.id, m.scope, m.project_id, p.name, m.source_client,
                m.memory_type, m.title, m.summary, m.status, m.files_json, m.tags_json, m.updated_at
         FROM memories m
         LEFT JOIN projects p ON p.id = m.project_id
         WHERE (?1 IS NULL OR m.project_id = ?1) AND m.created_at > ?2
         ORDER BY m.created_at ASC LIMIT ?3"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![project_id, anchor_time, limit as i64], |r| {
        let files_json: String = r.get(9)?;
        let tags_json: String = r.get(10)?;
        Ok(MemoryHit {
            id: r.get(0)?,
            scope: r.get(1)?,
            project_id: r.get(2)?,
            project_name: r.get(3)?,
            source_client: r.get(4)?,
            memory_type: r.get(5)?,
            title: r.get(6)?,
            summary: r.get(7)?,
            status: r.get(8)?,
            files: serde_json::from_str(&files_json).unwrap_or_default(),
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            updated_at: r.get(11)?,
            score: 1.0,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn sanitize_fts_query(raw: &str) -> Option<String> {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .take(12)
        .map(|t| format!("\"{t}\"*"))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

fn vec_to_le_bytes(vec: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for &f in vec {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

fn row_to_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: r.get(0)?,
        project_id: r.get(1)?,
        source_client: r.get(2)?,
        external_session_id: r.get(3)?,
        branch: r.get(4)?,
        started_at: r.get(5)?,
        ended_at: r.get(6)?,
        last_turn_at: r.get(7)?,
        status: r.get(8)?,
    })
}

fn row_to_raw_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawEvent> {
    Ok(RawEvent {
        id: r.get(0)?,
        session_id: r.get(1)?,
        project_id: r.get(2)?,
        source_client: r.get(3)?,
        event_type: r.get(4)?,
        tool_name: r.get(5)?,
        payload_json: r.get(6)?,
        distilled: r.get::<_, i64>(7)? != 0,
        created_at: r.get(8)?,
    })
}

fn row_to_memory(r: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    let files_json: String = r.get(10)?;
    let tags_json: String = r.get(11)?;
    Ok(Memory {
        id: r.get(0)?,
        scope: r.get(1)?,
        project_id: r.get(2)?,
        project_name: r.get(3)?,
        session_id: r.get(4)?,
        source_client: r.get(5)?,
        memory_type: r.get(6)?,
        title: r.get(7)?,
        summary: r.get(8)?,
        content: r.get(9)?,
        files: serde_json::from_str(&files_json).unwrap_or_default(),
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        status: r.get(12)?,
        superseded_by: r.get(13)?,
        created_at: r.get(14)?,
        updated_at: r.get(15)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_vec(idx: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; EMBEDDING_DIM];
        v[idx % EMBEDDING_DIM] = 1.0;
        v
    }

    #[test]
    fn hybrid_fts_and_vector_rrf_plus_supersede() {
        let db = MemoryDb::open_in_memory().unwrap();
        let proj = ProjectIdentity {
            id: "proj_demo".into(),
            name: "demo".into(),
            remote_url: Some("github.com/u/demo".into()),
            root_path: "/tmp/demo".into(),
            branch: Some("main".into()),
        };
        db.upsert_project(&proj).unwrap();

        let m1 = db
            .create_memory(
                NewMemoryInput {
                    scope: Some("project".into()),
                    project_id: Some("proj_demo".into()),
                    session_id: None,
                    source_client: Some("claude-code".into()),
                    memory_type: "decision".into(),
                    title: "Store auth tokens in localStorage".into(),
                    summary: "Initial JWT auth decision".into(),
                    content: "Stored JWT in localStorage temporarily".into(),
                    files: Some(vec!["src/auth.ts".into()]),
                    tags: Some(vec!["auth".into()]),
                    status: None,
                    supersedes_id: None,
                },
                Some(&unit_vec(1)),
            )
            .unwrap();

        let m2 = db
            .create_memory(
                NewMemoryInput {
                    scope: Some("project".into()),
                    project_id: Some("proj_demo".into()),
                    session_id: None,
                    source_client: Some("codex".into()),
                    memory_type: "decision".into(),
                    title: "Migrated session auth to httpOnly cookies".into(),
                    summary: "Replaced localStorage JWT with secure httpOnly cookies".into(),
                    content: "Switched login persistence to httpOnly cookies to prevent XSS".into(),
                    files: Some(vec!["src/auth.ts".into()]),
                    tags: Some(vec!["auth".into(), "security".into()]),
                    status: Some("pinned".into()),
                    supersedes_id: Some(m1.id.clone()),
                },
                Some(&unit_vec(2)),
            )
            .unwrap();

        // Superseded memory excluded by default; vector match finds m2 even with different wording
        let hits = db
            .search_memories(
                &SearchFilter {
                    query: Some("login persistence".into()),
                    project_id: Some("proj_demo".into()),
                    ..Default::default()
                },
                Some(&unit_vec(2)),
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, m2.id);
        assert_eq!(hits[0].source_client, "codex");

        // Briefing respects compact budget and shows cross-client pinned entry
        let (briefing, count) = db.build_session_briefing(Some("proj_demo"), 400).unwrap();
        assert_eq!(count, 1);
        assert!(briefing.contains("httpOnly cookies"));
        assert!(briefing.contains(&m2.id));

        // Regression test: memory_timeline must not deadlock on self.conn Mutex
        let timeline = db.memory_timeline(&m2.id, 3, 3).unwrap();
        assert!(timeline.iter().any(|h| h.id == m2.id));
    }

    #[test]
    fn onboarding_seed_auto_skips_upgrading_users_without_overwriting_explicit_false() {
        // Fresh user stays incomplete
        let fresh = MemoryDb::open_in_memory().unwrap();
        assert!(!fresh.seed_onboarding_if_existing(false).unwrap());
        assert!(!fresh.get_settings().unwrap().onboarding_completed);

        // Upgrading user with detected client config (even on mismatched port) auto-skips
        let upgraded_client = MemoryDb::open_in_memory().unwrap();
        assert!(upgraded_client.seed_onboarding_if_existing(true).unwrap());
        assert!(upgraded_client.get_settings().unwrap().onboarding_completed);

        // Upgrading user with existing memory auto-skips
        let upgraded_mem = MemoryDb::open_in_memory().unwrap();
        upgraded_mem
            .create_memory(
                NewMemoryInput {
                    scope: Some("global".into()),
                    project_id: None,
                    session_id: None,
                    source_client: Some("ui".into()),
                    memory_type: "pattern".into(),
                    title: "Existing".into(),
                    summary: "Prior v0.1.2 memory".into(),
                    content: "Stored before onboarding existed".into(),
                    files: None,
                    tags: None,
                    status: None,
                    supersedes_id: None,
                },
                None,
            )
            .unwrap();
        assert!(upgraded_mem.seed_onboarding_if_existing(false).unwrap());
        assert!(upgraded_mem.get_settings().unwrap().onboarding_completed);

        // Explicit onboarding_completed=false is preserved
        let mut s = upgraded_mem.get_settings().unwrap();
        s.onboarding_completed = false;
        upgraded_mem.save_settings(&s).unwrap();
        assert!(!upgraded_mem.seed_onboarding_if_existing(true).unwrap());
        assert!(!upgraded_mem.get_settings().unwrap().onboarding_completed);
    }
}
