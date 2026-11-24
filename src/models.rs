use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub remote_url: Option<String>,
    pub root_paths: Vec<String>,
    pub privacy_mode: String, // "full" | "local_only" | "paused"
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub source_client: String,
    pub external_session_id: String,
    pub branch: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub last_turn_at: String,
    pub status: String, // "active" | "ended"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub id: String,
    pub session_id: String,
    pub project_id: String,
    pub source_client: String,
    pub event_type: String, // "prompt" | "tool_use" | "tool_error" | "turn_stop" | "t3_message"
    pub tool_name: Option<String>,
    pub payload_json: String,
    pub distilled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub scope: String, // "project" | "global"
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub session_id: Option<String>,
    pub source_client: String, // "claude-code" | "codex" | "antigravity" | "t3-code" | "mcp" | "ui"
    pub memory_type: String, // "decision" | "bugfix" | "architecture" | "preference" | "pattern" | "session_summary"
    pub title: String,
    pub summary: String,
    pub content: String,
    pub files: Vec<String>,
    pub tags: Vec<String>,
    pub status: String, // "active" | "pinned" | "superseded" | "archived"
    pub superseded_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHit {
    pub id: String,
    pub scope: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub source_client: String,
    pub memory_type: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub files: Vec<String>,
    pub tags: Vec<String>,
    pub updated_at: String,
    pub score: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewMemoryInput {
    pub scope: Option<String>,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub source_client: Option<String>,
    pub memory_type: String,
    pub title: String,
    pub summary: String,
    pub content: String,
    pub files: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
    pub supersedes_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateMemoryInput {
    pub status: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub files: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub superseded_by: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchFilter {
    pub query: Option<String>,
    pub project_id: Option<String>,
    pub scope: Option<String>,
    pub memory_type: Option<String>,
    pub source_client: Option<String>,
    pub include_inactive: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub operation: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub estimated_cost_usd: f64,
    pub session_id: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSummary {
    pub today_input_tokens: i64,
    pub today_output_tokens: i64,
    pub today_total_tokens: i64,
    pub today_cost_usd: f64,
    pub today_calls: i64,
    pub today_blocked: i64,
    pub total_memories: i64,
    pub total_projects: i64,
    pub pending_raw_events: i64,
    pub daily_token_cap: i64,
    pub daily_cost_cap_usd: f64,
    pub budget_exhausted: bool,
    pub active_provider: String,
    pub active_model: String,
    pub embedding_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConnection {
    pub client_id: String,
    pub display_name: String,
    pub detected: bool,
    pub connected: bool,
    pub needs_manual_step: bool,
    pub manual_step_hint: Option<String>,
    pub config_paths: Vec<String>,
    pub config_backup_path: Option<String>,
    pub last_heartbeat_at: Option<String>,
    pub last_event_at: Option<String>,
    pub status_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub kind: String, // "memory_created" | "memory_updated" | "memory_deleted" | "raw_event" | "usage_recorded" | "connection_changed"
    pub timestamp: String,
    pub payload: serde_json::Value,
}

pub fn normalize_memory_type(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "decision" | "decisions" => "decision",
        "bugfix" | "bug" | "fix" => "bugfix",
        "architecture" | "arch" | "design" => "architecture",
        "preference" | "pref" | "style" | "convention" => "preference",
        "pattern" | "gotcha" | "discovery" => "pattern",
        "session_summary" | "summary" => "session_summary",
        _ => "pattern",
    }
}

pub fn normalize_scope(raw: Option<&str>, project_id: Option<&str>) -> &'static str {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("global") => "global",
        Some("project") => "project",
        _ => {
            if project_id.is_some() {
                "project"
            } else {
                "global"
            }
        }
    }
}

pub fn normalize_status(raw: Option<&str>) -> &'static str {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("pinned") => "pinned",
        Some("superseded") => "superseded",
        Some("archived") => "archived",
        _ => "active",
    }
}

pub fn normalize_privacy_mode(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "local_only" | "local-only" | "local" => "local_only",
        "paused" | "pause" | "off" => "paused",
        _ => "full",
    }
}

pub fn normalize_client(raw: Option<&str>) -> String {
    match raw
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
        .unwrap_or("mcp")
    {
        "claude" | "claude-code" | "claude_code" => "claude-code".into(),
        "codex" | "codex-cli" | "codex-app" => "codex".into(),
        "antigravity" | "agy" | "gemini" => "antigravity".into(),
        "t3" | "t3-code" | "t3code" => "t3-code".into(),
        "ui" | "user" => "ui".into(),
        other if !other.is_empty() => other.to_string(),
        _ => "mcp".into(),
    }
}
