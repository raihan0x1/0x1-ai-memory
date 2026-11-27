#![deny(clippy::string_slice)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 10101;
pub const EMBEDDING_DIM: usize = 384;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub backups_dir: PathBuf,
    pub models_dir: PathBuf,
    pub pid_path: PathBuf,
    pub log_path: PathBuf,
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
            log_path: data_dir.join("daemon.log"),
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

pub fn write_atomic_file(path: &Path, content: &str) -> Result<()> {
    ensure_parent_dir(path)?;
    let tmp = path.with_extension("ox1-tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
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
    pub distill_idle_seconds: i64,
    pub onboarding_completed: bool,
    pub auto_update_check: bool,
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
            distill_idle_seconds: 120,
            onboarding_completed: false,
            auto_update_check: true,
        }
    }
}

impl Settings {
    /// Offline-safe settings for unit and integration tests (never downloads ONNX models).
    pub fn for_testing() -> Self {
        Self {
            embedding_mode: "fts_only".into(),
            ..Self::default()
        }
    }

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
        if let Some(v) = map.get("distill_idle_seconds").and_then(|x| x.parse().ok()) {
            s.distill_idle_seconds = v;
        }
        if let Some(v) = map.get("onboarding_completed") {
            s.onboarding_completed = v == "true" || v == "1";
        }
        if let Some(v) = map.get("auto_update_check") {
            s.auto_update_check = v == "true" || v == "1";
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
            self.daily_cost_cap_usd.to_string(),
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
            "distill_idle_seconds".into(),
            self.distill_idle_seconds.to_string(),
        );
        m.insert(
            "onboarding_completed".into(),
            if self.onboarding_completed {
                "true"
            } else {
                "false"
            }
            .into(),
        );
        m.insert(
            "auto_update_check".into(),
            if self.auto_update_check {
                "true"
            } else {
                "false"
            }
            .into(),
        );
        m
    }

    pub fn effective_base_url(&self) -> String {
        if !self.base_url.trim().is_empty() {
            return self.base_url.trim().trim_end_matches('/').to_string();
        }
        self.default_base_url().to_string()
    }

    fn default_base_url(&self) -> &'static str {
        match self.provider.as_str() {
            "openrouter" => "https://openrouter.ai/api/v1",
            "ollama" => "http://127.0.0.1:11434/v1",
            "gemini" => "https://generativelanguage.googleapis.com/v1beta",
            _ => "https://api.openai.com/v1",
        }
    }

    /// Resolves an explicit key first. Implicit provider env keys apply only to the default endpoint.
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
        if self.effective_base_url() != self.default_base_url() {
            return None;
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

    /// True if dispatching to the active provider sends data off-machine (APIs or CLI tools).
    pub fn is_external_destination(&self) -> bool {
        match self.provider.as_str() {
            "none" => false,
            "ollama" | "openai_compatible" => !self.is_local_base_url(),
            _ => true,
        }
    }

    pub fn is_local_base_url(&self) -> bool {
        reqwest::Url::parse(&self.effective_base_url()).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        })
    }

    pub fn redacted(&self) -> SettingsView {
        let has_key = self.resolve_api_key().is_some();
        let trimmed = self.api_key.trim();
        let char_count = trimmed.chars().count();
        let masked = if trimmed.starts_with("env:") {
            trimmed.to_string()
        } else if char_count >= 8 {
            let prefix: String = trimmed.chars().take(4).collect();
            let suffix: String = trimmed
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("{prefix}...{suffix}")
        } else if !trimmed.is_empty() {
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
            distill_idle_seconds: self.distill_idle_seconds,
            onboarding_completed: self.onboarding_completed,
            auto_update_check: self.auto_update_check,
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
    pub distill_idle_seconds: i64,
    pub onboarding_completed: bool,
    pub auto_update_check: bool,
    pub zero_silent_fallback: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsPatch {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// Empty clears the key. Omission keeps it only when the provider and endpoint stay the same.
    pub api_key: Option<String>,
    pub daily_token_cap: Option<i64>,
    pub daily_cost_cap_usd: Option<f64>,
    pub session_start_token_budget: Option<usize>,
    pub embedding_mode: Option<String>,
    pub embedding_model: Option<String>,
    pub raw_event_retention_days: Option<i64>,
    pub distill_idle_seconds: Option<i64>,
    pub onboarding_completed: Option<bool>,
    pub auto_update_check: Option<bool>,
}

impl Settings {
    pub fn apply_patch(&mut self, patch: SettingsPatch) {
        let provider_changed = patch
            .provider
            .as_deref()
            .is_some_and(|provider| provider != self.provider);
        let previous_base_url = patch.base_url.as_ref().map(|_| self.effective_base_url());
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
        } else if provider_changed
            || previous_base_url.is_some_and(|previous| previous != self.effective_base_url())
        {
            self.api_key.clear();
        }
        if let Some(v) = patch.daily_token_cap {
            self.daily_token_cap = if v == 0 { 0 } else { v.max(1_000) };
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
        if let Some(v) = patch.distill_idle_seconds {
            self.distill_idle_seconds = v.clamp(15, 3600);
        }
        if let Some(v) = patch.onboarding_completed {
            self.onboarding_completed = v;
        }
        if let Some(v) = patch.auto_update_check {
            self.auto_update_check = v;
        }
    }
}

#[derive(Clone)]
pub(crate) struct ProviderHttp {
    remote: reqwest::Client,
    loopback: reqwest::Client,
}

impl ProviderHttp {
    pub(crate) fn new(timeout: std::time::Duration) -> Self {
        let builder = || {
            reqwest::Client::builder()
                .timeout(timeout)
                .redirect(provider_redirect_policy())
        };
        Self {
            remote: builder().build().expect("initialize provider HTTP client"),
            loopback: builder()
                .no_proxy()
                .build()
                .expect("initialize direct provider HTTP client"),
        }
    }

    pub(crate) fn for_settings(&self, settings: &Settings) -> &reqwest::Client {
        if settings.is_local_base_url() {
            &self.loopback
        } else {
            &self.remote
        }
    }
}

fn provider_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt
            .previous()
            .last()
            .is_some_and(|previous| previous.origin() != attempt.url().origin())
        {
            return attempt.error("Provider redirected the request to a different origin");
        }
        reqwest::redirect::Policy::default().redirect(attempt)
    })
}

pub fn is_localhost_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let without_scheme = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
        .unwrap_or(&lower);
    let host = without_scheme.split('/').next().unwrap_or("");
    host == "127.0.0.1"
        || host.starts_with("127.0.0.1:")
        || host == "localhost"
        || host.starts_with("localhost:")
        || host == "[::1]"
        || host.starts_with("[::1]:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_multibyte_utf8_api_key_without_panicking() {
        let s = Settings {
            api_key: "••••••••••".into(),
            ..Default::default()
        };
        let view = s.redacted();
        assert_eq!(view.api_key_masked, "••••...••••");
    }

    #[test]
    fn identifies_external_destinations_including_cli_providers() {
        let mut s = Settings::default();
        assert!(!s.is_external_destination());
        s.provider = "ollama".into();
        assert!(!s.is_external_destination());
        s.base_url = "https://ollama.example/v1".into();
        assert!(s.is_external_destination());
        s.base_url = "http://localhost:11434@ollama.example/v1".into();
        assert!(s.is_external_destination());
        s.base_url = String::new();
        s.provider = "cli_claude".into();
        assert!(s.is_external_destination());
        s.provider = "openrouter".into();
        assert!(s.is_external_destination());
        s.provider = "openai_compatible".into();
        s.base_url = "http://127.0.0.1:1234/v1".into();
        assert!(!s.is_external_destination());
    }

    #[test]
    fn retains_saved_key_only_for_the_same_provider_and_endpoint() {
        let mut settings = Settings {
            provider: "openrouter".into(),
            api_key: "env:MEMORY_OPENROUTER_KEY".into(),
            ..Settings::default()
        };
        let patch = serde_json::from_value(serde_json::json!({
            "model": "another-model",
            "base_url": " https://openrouter.ai/api/v1/ "
        }))
        .unwrap();
        settings.apply_patch(patch);
        assert_eq!(settings.api_key, "env:MEMORY_OPENROUTER_KEY");

        let patch = serde_json::from_value(serde_json::json!({
            "base_url": "https://custom.example/v1"
        }))
        .unwrap();
        settings.apply_patch(patch);
        assert!(settings.api_key.is_empty());
        assert!(settings.resolve_api_key().is_none());

        let patch = serde_json::from_value(serde_json::json!({
            "provider": "openai_compatible",
            "api_key": "synthetic-custom-key"
        }))
        .unwrap();
        settings.apply_patch(patch);
        assert_eq!(settings.api_key, "synthetic-custom-key");

        let patch = serde_json::from_value(serde_json::json!({ "provider": "ollama" })).unwrap();
        settings.apply_patch(patch);
        assert!(settings.api_key.is_empty());
    }

    #[tokio::test]
    async fn provider_redirects_are_limited_to_the_same_origin() {
        use axum::{Router, response::Redirect, routing::get};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let other_origin = format!("http://localhost:{}/answer", address.port());
        let router = Router::new()
            .route("/answer", get(|| async { "ok" }))
            .route(
                "/same-origin",
                get(|| async { Redirect::temporary("/answer") }),
            )
            .route(
                "/other-origin",
                get(move || async move { Redirect::temporary(&other_origin) }),
            );
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .redirect(provider_redirect_policy())
            .build()
            .unwrap();
        let url = format!("http://{address}");
        let response = http.get(format!("{url}/same-origin")).send().await.unwrap();
        assert_eq!(response.text().await.unwrap(), "ok");
        let error = http
            .get(format!("{url}/other-origin"))
            .send()
            .await
            .unwrap_err();
        assert!(error.is_redirect());
        task.abort();
    }

    #[test]
    fn default_port_is_10101_with_log_path_and_onboarding_flag() {
        assert_eq!(DEFAULT_PORT, 10101);
        let paths = AppPaths::from_dir(PathBuf::from("/tmp/ox1-test"));
        assert_eq!(paths.log_path, PathBuf::from("/tmp/ox1-test/daemon.log"));

        let mut s = Settings::default();
        assert!(!s.onboarding_completed);
        s.apply_patch(SettingsPatch {
            provider: None,
            model: None,
            base_url: None,
            api_key: None,
            daily_token_cap: None,
            daily_cost_cap_usd: None,
            session_start_token_budget: None,
            embedding_mode: None,
            embedding_model: None,
            raw_event_retention_days: None,
            distill_idle_seconds: None,
            onboarding_completed: Some(true),
            auto_update_check: Some(false),
        });
        assert!(s.onboarding_completed);
        assert!(!s.auto_update_check);
        let roundtrip = Settings::from_map(&s.to_map());
        assert!(roundtrip.onboarding_completed);
        assert!(roundtrip.redacted().onboarding_completed);
        assert!(!roundtrip.auto_update_check);
        assert!(!roundtrip.redacted().auto_update_check);
    }
}
