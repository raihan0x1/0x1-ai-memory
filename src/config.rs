use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 41777;
pub const EMBEDDING_DIM: usize = 384;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub backups_dir: PathBuf,
    pub models_dir: PathBuf,
    pub pid_path: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<Self> {
        let data_dir = if let Ok(custom) = std::env::var("OX1_MEMORY_DATA_DIR") {
            PathBuf::from(custom)
        } else {
            let home = dirs::home_dir().context("Could not determine home directory")?;
            home.join(".0x1-memory")
        };
        Ok(Self::from_dir(data_dir))
    }

    pub fn from_dir(data_dir: PathBuf) -> Self {
        Self {
            db_path: data_dir.join("memory.db"),
            backups_dir: data_dir.join("backups"),
            models_dir: data_dir.join("models"),
            pid_path: data_dir.join("daemon.pid"),
            data_dir,
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.backups_dir)?;
        std::fs::create_dir_all(&self.models_dir)?;
        Ok(())
    }
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub daily_token_cap: i64,
    pub daily_cost_cap_usd: f64,
    pub session_start_token_budget: usize,
    pub embedding_mode: String,
    pub embedding_model: String,
    pub raw_event_retention_days: i64,
    pub t3_watcher_enabled: bool,
    pub distill_idle_seconds: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            model: "google/gemini-2.5-flash-lite".into(),
            base_url: String::new(),
            api_key: String::new(),
            daily_token_cap: 250_000,
            daily_cost_cap_usd: 0.50,
            session_start_token_budget: 450,
            embedding_mode: "local_onnx".into(),
            embedding_model: "all-MiniLM-L6-v2".into(),
            raw_event_retention_days: 14,
            t3_watcher_enabled: true,
            distill_idle_seconds: 120,
        }
    }
}

impl Settings {
    pub fn from_map(map: &HashMap<String, String>) -> Self {
        let mut s = Self::default();
        if let Some(v) = map.get("provider") {
            s.provider = v.clone();
        }
        if let Some(v) = map.get("model") {
            s.model = v.clone();
        }
        if let Some(v) = map.get("base_url") {
            s.base_url = v.clone();
        }
        if let Some(v) = map.get("api_key") {
            s.api_key = v.clone();
        }
        if let Some(v) = map.get("daily_token_cap").and_then(|x| x.parse().ok()) {
            s.daily_token_cap = v;
        }
        if let Some(v) = map.get("daily_cost_cap_usd").and_then(|x| x.parse().ok()) {
            s.daily_cost_cap_usd = v;
        }
        if let Some(v) = map
            .get("session_start_token_budget")
            .and_then(|x| x.parse().ok())
        {
            s.session_start_token_budget = v;
        }
        if let Some(v) = map.get("embedding_mode") {
            s.embedding_mode = v.clone();
        }
        if let Some(v) = map.get("embedding_model") {
            s.embedding_model = v.clone();
        }
        if let Some(v) = map
            .get("raw_event_retention_days")
            .and_then(|x| x.parse().ok())
        {
            s.raw_event_retention_days = v;
        }
        if let Some(v) = map.get("t3_watcher_enabled") {
            s.t3_watcher_enabled = v == "true" || v == "1";
        }
        if let Some(v) = map.get("distill_idle_seconds").and_then(|x| x.parse().ok()) {
            s.distill_idle_seconds = v;
        }
        s
    }

    pub fn to_map(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("provider".into(), self.provider.clone());
        m.insert("model".into(), self.model.clone());
        m.insert("base_url".into(), self.base_url.clone());
        m.insert("api_key".into(), self.api_key.clone());
        m.insert("daily_token_cap".into(), self.daily_token_cap.to_string());
        m.insert(
            "daily_cost_cap_usd".into(),
            format!("{:.4}", self.daily_cost_cap_usd),
        );
        m.insert(
            "session_start_token_budget".into(),
            self.session_start_token_budget.to_string(),
        );
        m.insert("embedding_mode".into(), self.embedding_mode.clone());
        m.insert("embedding_model".into(), self.embedding_model.clone());
        m.insert(
            "raw_event_retention_days".into(),
            self.raw_event_retention_days.to_string(),
        );
        m.insert(
            "t3_watcher_enabled".into(),
            if self.t3_watcher_enabled {
                "true"
            } else {
                "false"
            }
            .into(),
        );
        m.insert(
            "distill_idle_seconds".into(),
            self.distill_idle_seconds.to_string(),
        );
        m
    }

    pub fn effective_base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            return self.base_url.trim().trim_end_matches('/').to_string();
        }
        match self.provider.as_str() {
            "openrouter" => "https://openrouter.ai/api/v1".into(),
            "ollama" => "http://127.0.0.1:11434/v1".into(),
            "gemini" => "https://generativelanguage.googleapis.com/v1beta".into(),
            _ => "https://api.openai.com/v1".into(),
        }
    }

    /// Resolves `env:VAR_NAME`, stored key, or standard provider env var.
    pub fn resolve_api_key(&self) -> Option<String> {
        let raw = self.api_key.trim();
        if let Some(env_name) = raw.strip_prefix("env:") {
            return std::env::var(env_name.trim())
                .ok()
                .filter(|v| !v.trim().is_empty());
        }
        if !raw.is_empty() {
            return Some(raw.to_string());
        }
        let fallback_env = match self.provider.as_str() {
            "openrouter" => Some("OPENROUTER_API_KEY"),
            "gemini" => Some("GEMINI_API_KEY"),
            "openai_compatible" => Some("OPENAI_API_KEY"),
            _ => None,
        };
        fallback_env
            .and_then(|k| std::env::var(k).ok())
            .filter(|v| !v.trim().is_empty())
    }

    pub fn is_cli_provider(&self) -> bool {
        matches!(
            self.provider.as_str(),
            "cli_claude" | "cli_codex" | "cli_agy"
        )
    }

    pub fn is_external_api(&self) -> bool {
        matches!(
            self.provider.as_str(),
            "openrouter" | "gemini" | "openai_compatible"
        )
    }

    pub fn redacted(&self) -> SettingsView {
        let has_key = self.resolve_api_key().is_some();
        let masked = if self.api_key.trim().starts_with("env:") {
            self.api_key.trim().to_string()
        } else if self.api_key.trim().len() >= 8 {
            let trimmed = self.api_key.trim();
            format!("{}...{}", &trimmed[..4], &trimmed[trimmed.len() - 4..])
        } else if !self.api_key.trim().is_empty() {
            "••••••••".into()
        } else {
            String::new()
        };
        SettingsView {
            provider: self.provider.clone(),
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            api_key_masked: masked,
            has_api_key: has_key,
            daily_token_cap: self.daily_token_cap,
            daily_cost_cap_usd: self.daily_cost_cap_usd,
            session_start_token_budget: self.session_start_token_budget,
            embedding_mode: self.embedding_mode.clone(),
            embedding_model: self.embedding_model.clone(),
            raw_event_retention_days: self.raw_event_retention_days,
            t3_watcher_enabled: self.t3_watcher_enabled,
            distill_idle_seconds: self.distill_idle_seconds,
            zero_silent_fallback: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsView {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key_masked: String,
    pub has_api_key: bool,
    pub daily_token_cap: i64,
    pub daily_cost_cap_usd: f64,
    pub session_start_token_budget: usize,
    pub embedding_mode: String,
    pub embedding_model: String,
    pub raw_event_retention_days: i64,
    pub t3_watcher_enabled: bool,
    pub distill_idle_seconds: i64,
    pub zero_silent_fallback: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsPatch {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// Omitted or `None` keeps existing key; empty string clears stored key.
    pub api_key: Option<String>,
    pub daily_token_cap: Option<i64>,
    pub daily_cost_cap_usd: Option<f64>,
    pub session_start_token_budget: Option<usize>,
    pub embedding_mode: Option<String>,
    pub embedding_model: Option<String>,
    pub raw_event_retention_days: Option<i64>,
    pub t3_watcher_enabled: Option<bool>,
    pub distill_idle_seconds: Option<i64>,
}

impl Settings {
    pub fn apply_patch(&mut self, patch: SettingsPatch) {
        if let Some(v) = patch.provider {
            self.provider = v;
        }
        if let Some(v) = patch.model {
            self.model = v;
        }
        if let Some(v) = patch.base_url {
            self.base_url = v;
        }
        if let Some(v) = patch.api_key {
            self.api_key = v;
        }
        if let Some(v) = patch.daily_token_cap {
            self.daily_token_cap = v.max(1_000);
        }
        if let Some(v) = patch.daily_cost_cap_usd {
            self.daily_cost_cap_usd = v.max(0.0);
        }
        if let Some(v) = patch.session_start_token_budget {
            self.session_start_token_budget = v.clamp(100, 2_000);
        }
        if let Some(v) = patch.embedding_mode {
            self.embedding_mode = v;
        }
        if let Some(v) = patch.embedding_model {
            self.embedding_model = v;
        }
        if let Some(v) = patch.raw_event_retention_days {
            self.raw_event_retention_days = v.clamp(1, 365);
        }
        if let Some(v) = patch.t3_watcher_enabled {
            self.t3_watcher_enabled = v;
        }
        if let Some(v) = patch.distill_idle_seconds {
            self.distill_idle_seconds = v.clamp(15, 3600);
        }
    }
}
