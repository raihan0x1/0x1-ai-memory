#![deny(clippy::string_slice)]

use crate::db::MemoryDb;
use crate::models::StreamEvent;
use anyhow::{Context, Result, bail};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::{Mutex, broadcast, watch};

pub const GITHUB_REPO: &str = "raihan0x1/0x1-ai-memory";
pub const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";
pub const DEFAULT_GITHUB_RELEASES_BASE: &str =
    "https://github.com/raihan0x1/0x1-ai-memory/releases/download";
pub const UPDATE_CHECK_INTERVAL_SECS: i64 = 6 * 3600;

const META_LAST_SEEN_VERSION: &str = "last_seen_version";
const META_PREVIOUS_VERSION: &str = "previous_version";
const META_DISMISSED_VERSION: &str = "update_dismissed_version";
const META_LAST_CHECKED_AT: &str = "update_last_checked_at";
const META_UPDATE_ETAG: &str = "update_etag";
const META_UPDATE_LATEST_JSON: &str = "update_latest_json";
const META_UPDATE_CHECK_ERROR: &str = "update_check_error";
const META_WHATS_NEW_CACHE_JSON: &str = "whats_new_cache_json";

#[cfg(windows)]
const BIN_NAME: &str = "0x1-memory.exe";
#[cfg(not(windows))]
const BIN_NAME: &str = "0x1-memory";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfUpdateCapability {
    pub supported: bool,
    pub reason: Option<String>,
    pub install_kind: String, // "npx" | "binary" | "dev_build" | "unsupported_platform"
    pub manual_command: String,
    pub asset_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateRuntimeState {
    pub in_progress: bool,
    pub stage: String, // "idle" | "downloading" | "verifying" | "extracting" | "smoke_testing" | "replacing" | "restarting" | "error"
    pub target_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsNewInfo {
    pub should_show: bool,
    pub version: String,
    pub previous_version: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub html_url: String,
    pub compare_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatusResponse {
    pub current_version: String,
    pub auto_check_enabled: bool,
    pub last_checked_at: Option<String>,
    pub check_error: Option<String>,
    pub update_available: bool,
    pub dismissed_version: Option<String>,
    pub latest_release: Option<ReleaseInfo>,
    pub capability: SelfUpdateCapability,
    pub runtime: UpdateRuntimeState,
    pub whats_new: WhatsNewInfo,
}

#[derive(Debug, Deserialize)]
struct GitHubReleasePayload {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

impl GitHubReleasePayload {
    fn into_release_info(self) -> Option<ReleaseInfo> {
        let version = self.tag_name.trim().trim_start_matches('v').to_string();
        parse_semver(&version)?;
        let tag_name = self.tag_name.trim().to_string();
        let html_url = self
            .html_url
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("https://github.com/{GITHUB_REPO}/releases/tag/{tag_name}"));
        let name = self
            .name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| tag_name.clone());
        Some(ReleaseInfo {
            tag_name,
            version,
            name,
            body: self.body.unwrap_or_default(),
            html_url,
            published_at: self.published_at,
        })
    }
}

#[derive(Clone)]
pub struct UpdateManager {
    db: MemoryDb,
    exe_path: PathBuf,
    data_dir: PathBuf,
    log_path: PathBuf,
    port: u16,
    shutdown_tx: watch::Sender<bool>,
    events_tx: broadcast::Sender<StreamEvent>,
    current_version: String,
    api_base: String,
    releases_base: String,
    runtime: Arc<Mutex<UpdateRuntimeState>>,
}

impl UpdateManager {
    pub fn new(
        db: MemoryDb,
        exe_path: PathBuf,
        data_dir: PathBuf,
        log_path: PathBuf,
        port: u16,
        shutdown_tx: watch::Sender<bool>,
        events_tx: broadcast::Sender<StreamEvent>,
    ) -> Self {
        Self::with_urls_and_version(
            db,
            exe_path,
            data_dir,
            log_path,
            port,
            shutdown_tx,
            events_tx,
            env!("CARGO_PKG_VERSION").to_string(),
            DEFAULT_GITHUB_API_BASE.to_string(),
            DEFAULT_GITHUB_RELEASES_BASE.to_string(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_urls_and_version(
        db: MemoryDb,
        exe_path: PathBuf,
        data_dir: PathBuf,
        log_path: PathBuf,
        port: u16,
        shutdown_tx: watch::Sender<bool>,
        events_tx: broadcast::Sender<StreamEvent>,
        current_version: String,
        api_base: String,
        releases_base: String,
    ) -> Self {
        Self {
            db,
            exe_path,
            data_dir,
            log_path,
            port,
            shutdown_tx,
            events_tx,
            current_version: current_version.trim_start_matches('v').to_string(),
            api_base: api_base.trim_end_matches('/').to_string(),
            releases_base: releases_base.trim_end_matches('/').to_string(),
            runtime: Arc::new(Mutex::new(UpdateRuntimeState {
                in_progress: false,
                stage: "idle".into(),
                target_version: None,
                error: None,
            })),
        }
    }

    /// Distinguishes fresh installs (record current version, skip "What's new")
    /// from upgrades (preserve old seen version so the UI shows "What's new" once).
    pub fn seed_version_state_on_startup(&self, has_configured_clients: bool) -> Result<()> {
        let existing_seen = self.db.get_meta(META_LAST_SEEN_VERSION)?;
        match existing_seen {
            None => {
                let has_data = self.db.has_existing_user_data().unwrap_or(false);
                if has_data || has_configured_clients {
                    self.db.set_meta(META_LAST_SEEN_VERSION, "0.0.0")?;
                } else {
                    self.db
                        .set_meta(META_LAST_SEEN_VERSION, &self.current_version)?;
                }
            }
            Some(seen) if is_newer_version(&seen, &self.current_version) => {
                if seen != "0.0.0" {
                    let _ = self.db.set_meta(META_PREVIOUS_VERSION, &seen);
                }
            }
            Some(_) => {}
        }
        Ok(())
    }

    pub async fn cleanup_old_binary_on_startup(&self) {
        let old_path = old_binary_path(&self.exe_path);
        match tokio::fs::remove_file(&old_path).await {
            Ok(()) => tracing::info!("Cleaned up previous binary backup {}", old_path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::debug!("Could not remove {}: {e}", old_path.display()),
        }
    }

    pub async fn status(&self) -> UpdateStatusResponse {
        let exe_path = self.exe_path.clone();
        let data_dir = self.data_dir.clone();
        let capability = tokio::task::spawn_blocking(move || {
            detect_capability_for(&exe_path, &data_dir, platform_asset_name())
        })
        .await
        .unwrap_or_else(|_| SelfUpdateCapability {
            supported: false,
            reason: Some("Could not inspect binary permissions".into()),
            install_kind: "binary".into(),
            manual_command: default_manual_command("binary"),
            asset_name: platform_asset_name().map(ToOwned::to_owned),
        });

        let runtime = self.runtime.lock().await.clone();
        let settings = self.db.get_settings().unwrap_or_default();
        let last_checked_at = self
            .db
            .get_meta(META_LAST_CHECKED_AT)
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());
        let check_error = self
            .db
            .get_meta(META_UPDATE_CHECK_ERROR)
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());
        let dismissed_version = self
            .db
            .get_meta(META_DISMISSED_VERSION)
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());
        let latest_release = self
            .db
            .get_meta(META_UPDATE_LATEST_JSON)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<ReleaseInfo>(&raw).ok());

        let update_available = latest_release
            .as_ref()
            .is_some_and(|r| is_newer_stable_release(&self.current_version, &r.tag_name));

        let whats_new = self.build_whats_new(latest_release.as_ref());

        UpdateStatusResponse {
            current_version: self.current_version.clone(),
            auto_check_enabled: settings.auto_update_check,
            last_checked_at,
            check_error,
            update_available,
            dismissed_version,
            latest_release,
            capability,
            runtime,
            whats_new,
        }
    }

    fn build_whats_new(&self, latest_release: Option<&ReleaseInfo>) -> WhatsNewInfo {
        let last_seen = self
            .db
            .get_meta(META_LAST_SEEN_VERSION)
            .ok()
            .flatten()
            .unwrap_or_else(|| self.current_version.clone());
        let should_show = is_newer_version(&last_seen, &self.current_version);
        let previous_version = self
            .db
            .get_meta(META_PREVIOUS_VERSION)
            .ok()
            .flatten()
            .filter(|v| !v.is_empty() && v != "0.0.0" && v != &self.current_version)
            .or_else(|| {
                if last_seen != "0.0.0" && is_newer_version(&last_seen, &self.current_version) {
                    Some(last_seen.clone())
                } else {
                    None
                }
            });

        let cached_current = self
            .db
            .get_meta(META_WHATS_NEW_CACHE_JSON)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<ReleaseInfo>(&raw).ok())
            .filter(|r| r.version == self.current_version)
            .or_else(|| {
                latest_release
                    .filter(|r| r.version == self.current_version)
                    .cloned()
            });

        let tag = format!("v{}", self.current_version);
        let html_url = cached_current
            .as_ref()
            .map(|r| r.html_url.clone())
            .unwrap_or_else(|| format!("https://github.com/{GITHUB_REPO}/releases/tag/{tag}"));
        let compare_url = previous_version
            .as_ref()
            .map(|prev| {
                format!(
                    "https://github.com/{GITHUB_REPO}/compare/v{}...{tag}",
                    prev.trim_start_matches('v')
                )
            })
            .or_else(|| Some(html_url.clone()));

        WhatsNewInfo {
            should_show,
            version: self.current_version.clone(),
            previous_version,
            title: cached_current
                .as_ref()
                .map(|r| r.name.clone())
                .unwrap_or_else(|| format!("v{}", self.current_version)),
            body: cached_current
                .map(|r| r.body)
                .filter(|b| !b.trim().is_empty()),
            html_url,
            compare_url,
        }
    }

    pub async fn check_for_updates(&self, force: bool) -> UpdateStatusResponse {
        let settings = self.db.get_settings().unwrap_or_default();
        if !force {
            if !settings.auto_update_check {
                return self.status().await;
            }
            if let Ok(Some(last)) = self.db.get_meta(META_LAST_CHECKED_AT)
                && let Ok(ts) = DateTime::parse_from_rfc3339(&last)
            {
                let age_secs = (Utc::now() - ts.with_timezone(&Utc)).num_seconds();
                if age_secs < UPDATE_CHECK_INTERVAL_SECS {
                    return self.status().await;
                }
            }
        }

        let url = format!("{}/repos/{GITHUB_REPO}/releases/latest", self.api_base);
        let etag = self.db.get_meta(META_UPDATE_ETAG).ok().flatten();
        match self.fetch_github_release(&url, etag.as_deref()).await {
            Ok(Some((new_etag, payload))) => {
                let now = Utc::now().to_rfc3339();
                let _ = self.db.set_meta(META_LAST_CHECKED_AT, &now);
                let _ = self.db.set_meta(META_UPDATE_CHECK_ERROR, "");
                if let Some(et) = new_etag {
                    let _ = self.db.set_meta(META_UPDATE_ETAG, &et);
                }
                if !payload.draft
                    && !payload.prerelease
                    && let Some(info) = payload.into_release_info()
                    && let Ok(json) = serde_json::to_string(&info)
                {
                    let _ = self.db.set_meta(META_UPDATE_LATEST_JSON, &json);
                }
            }
            Ok(None) => {
                // 304 Not Modified: keep cached release JSON, update check timestamp
                let now = Utc::now().to_rfc3339();
                let _ = self.db.set_meta(META_LAST_CHECKED_AT, &now);
                let _ = self.db.set_meta(META_UPDATE_CHECK_ERROR, "");
            }
            Err(err) => {
                let _ = self.db.set_meta(META_UPDATE_CHECK_ERROR, &err.to_string());
            }
        }

        let status = self.status().await;
        self.emit_status_event(&status);
        status
    }

    pub async fn fetch_whats_new_for_current(&self) -> Result<UpdateStatusResponse> {
        let url = format!(
            "{}/repos/{GITHUB_REPO}/releases/tags/v{}",
            self.api_base, self.current_version
        );
        if let Some((_, payload)) = self.fetch_github_release(&url, None).await?
            && let Some(info) = payload.into_release_info()
            && let Ok(json) = serde_json::to_string(&info)
        {
            self.db.set_meta(META_WHATS_NEW_CACHE_JSON, &json)?;
        }
        let status = self.status().await;
        self.emit_status_event(&status);
        Ok(status)
    }

    pub async fn dismiss_version(&self, version: &str) -> Result<UpdateStatusResponse> {
        let clean = version.trim().trim_start_matches('v');
        self.db.set_meta(META_DISMISSED_VERSION, clean)?;
        let status = self.status().await;
        self.emit_status_event(&status);
        Ok(status)
    }

    pub async fn ack_whats_new(&self) -> Result<UpdateStatusResponse> {
        self.db
            .set_meta(META_LAST_SEEN_VERSION, &self.current_version)?;
        let status = self.status().await;
        self.emit_status_event(&status);
        Ok(status)
    }

    async fn fetch_github_release(
        &self,
        url: &str,
        etag: Option<&str>,
    ) -> Result<Option<(Option<String>, GitHubReleasePayload)>> {
        let client = build_http_client(&self.api_base, Duration::from_secs(10))?;
        let mut req = client
            .get(url)
            .header("User-Agent", format!("0x1-memory/{}", self.current_version))
            .header("Accept", "application/vnd.github+json");
        if let Some(et) = etag.filter(|s| !s.is_empty()) {
            req = req.header("If-None-Match", et);
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        if !resp.status().is_success() {
            bail!("GitHub release lookup returned HTTP {}", resp.status());
        }
        let new_etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        let payload: GitHubReleasePayload = resp.json().await?;
        Ok(Some((new_etag, payload)))
    }

    pub async fn start_self_update(&self) -> Result<UpdateStatusResponse, (StatusCode, String)> {
        let status_before = self.status().await;
        if !status_before.capability.supported {
            return Err((
                StatusCode::BAD_REQUEST,
                status_before
                    .capability
                    .reason
                    .unwrap_or_else(|| "Self-update is not supported for this install".into()),
            ));
        }
        let Some(target) = status_before.latest_release.clone() else {
            return Err((
                StatusCode::BAD_REQUEST,
                "No newer release cached; check for updates first".into(),
            ));
        };
        if !status_before.update_available {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Already running latest version v{}", self.current_version),
            ));
        }
        let Some(asset) = status_before.capability.asset_name.clone() else {
            return Err((
                StatusCode::BAD_REQUEST,
                "No prebuilt asset available for this platform".into(),
            ));
        };

        {
            let mut rt = self.runtime.lock().await;
            if rt.in_progress {
                return Err((
                    StatusCode::CONFLICT,
                    "An update is already in progress".into(),
                ));
            }
            *rt = UpdateRuntimeState {
                in_progress: true,
                stage: "downloading".into(),
                target_version: Some(target.version.clone()),
                error: None,
            };
        }

        let initial = self.status().await;
        self.emit_status_event(&initial);

        let mgr = self.clone();
        tokio::spawn(async move {
            if let Err(err) = mgr.stage_verify_and_swap(&target, &asset).await {
                mgr.set_runtime_error(&target.version, err.to_string())
                    .await;
                return;
            }

            mgr.set_runtime_stage("restarting", Some(&target.version))
                .await;
            tokio::time::sleep(Duration::from_millis(100)).await;

            if let Err(err) = mgr.spawn_updated_daemon().await {
                mgr.set_runtime_error(&target.version, err.to_string())
                    .await;
                return;
            }

            // Let HTTP/SSE responses flush cleanly before tearing down the old listener.
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _ = mgr.shutdown_tx.send(true);
        });

        Ok(initial)
    }

    pub async fn stage_verify_and_swap(&self, target: &ReleaseInfo, asset: &str) -> Result<()> {
        if asset.contains('/') || asset.contains('\\') || asset.contains("..") {
            bail!("Refusing invalid release asset name: {asset}");
        }
        let tag = target.tag_name.trim();
        let archive_url = format!("{}/{tag}/{asset}", self.releases_base);
        let sums_url = format!("{}/{tag}/SHA256SUMS.txt", self.releases_base);
        validate_release_download_url(&archive_url, &self.releases_base)?;
        validate_release_download_url(&sums_url, &self.releases_base)?;

        self.set_runtime_stage("downloading", Some(&target.version))
            .await;
        let archive_client = build_http_client(&self.releases_base, Duration::from_secs(120))?;
        let sums_client = build_http_client(&self.releases_base, Duration::from_secs(20))?;

        let sums_resp = sums_client
            .get(&sums_url)
            .header("User-Agent", format!("0x1-memory/{}", self.current_version))
            .send()
            .await
            .context("Failed to download SHA256SUMS.txt")?;
        if !sums_resp.status().is_success() {
            bail!(
                "SHA256SUMS.txt download returned HTTP {}",
                sums_resp.status()
            );
        }
        let sums_text = sums_resp.text().await?;

        let archive_resp = archive_client
            .get(&archive_url)
            .header("User-Agent", format!("0x1-memory/{}", self.current_version))
            .send()
            .await
            .with_context(|| format!("Failed to download {asset}"))?;
        if !archive_resp.status().is_success() {
            bail!("Archive download returned HTTP {}", archive_resp.status());
        }
        let archive_bytes = archive_resp.bytes().await?.to_vec();

        self.set_runtime_stage("verifying", Some(&target.version))
            .await;
        let expected = parse_sha256sums(&sums_text, asset)
            .with_context(|| format!("Checksum entry missing for {asset} in SHA256SUMS.txt"))?;
        let archive_for_hash = archive_bytes.clone();
        let actual = tokio::task::spawn_blocking(move || sha256_hex(&archive_for_hash))
            .await
            .context("SHA-256 verification task failed")?;
        if actual != expected {
            bail!("SHA-256 mismatch for {asset} (expected {expected}, got {actual})");
        }

        self.set_runtime_stage("extracting", Some(&target.version))
            .await;
        let exe_dir = self
            .exe_path
            .parent()
            .context("Could not resolve executable directory")?;
        tokio::fs::create_dir_all(exe_dir).await?;
        let tmp_dir = exe_dir.join(format!(
            ".ox1-update-{}-{}",
            std::process::id(),
            Utc::now().timestamp_millis()
        ));
        tokio::fs::create_dir_all(&tmp_dir).await?;

        let swap_result = self
            .extract_smoke_and_swap(&tmp_dir, exe_dir, asset, &archive_bytes, target)
            .await;
        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
        swap_result
    }

    async fn extract_smoke_and_swap(
        &self,
        tmp_dir: &Path,
        exe_dir: &Path,
        asset: &str,
        archive_bytes: &[u8],
        target: &ReleaseInfo,
    ) -> Result<()> {
        let archive_path = tmp_dir.join(asset);
        tokio::fs::write(&archive_path, archive_bytes).await?;

        let extract_dir = tmp_dir.join("extracted");
        tokio::fs::create_dir_all(&extract_dir).await?;
        extract_archive_async(&archive_path, &extract_dir).await?;

        let extracted = extract_dir.join(BIN_NAME);
        if !tokio::fs::try_exists(&extracted).await.unwrap_or(false) {
            bail!("Extracted archive did not contain {BIN_NAME}");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&extracted, std::fs::Permissions::from_mode(0o755)).await?;
        }

        self.set_runtime_stage("smoke_testing", Some(&target.version))
            .await;
        let smoke = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new(&extracted).arg("--version").output(),
        )
        .await
        .context("Smoke check timed out executing downloaded binary --version")?
        .context("Failed to execute downloaded binary for --version smoke check")?;
        if !smoke.status.success() {
            bail!(
                "Downloaded binary failed --version smoke check (status {})",
                smoke.status
            );
        }
        let out_text = format!(
            "{} {}",
            String::from_utf8_lossy(&smoke.stdout),
            String::from_utf8_lossy(&smoke.stderr)
        );
        if !out_text.contains(&target.version) {
            bail!(
                "Downloaded binary --version output '{}' did not match target v{}",
                out_text.trim(),
                target.version
            );
        }

        self.set_runtime_stage("replacing", Some(&target.version))
            .await;

        // Pre-cache release notes in SQLite before restart so "What's new" works even offline.
        if let Ok(json) = serde_json::to_string(target) {
            let _ = self.db.set_meta(META_WHATS_NEW_CACHE_JSON, &json);
        }

        let staged = exe_dir.join(format!("{BIN_NAME}.ox1-new-{}", std::process::id()));
        tokio::fs::copy(&extracted, &staged).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).await?;
        }

        if let Err(err) = replace_binary_file(&staged, &self.exe_path).await {
            let _ = tokio::fs::remove_file(&staged).await;
            return Err(err);
        }

        // Sync npx launcher .version stamp when running inside ~/.0x1-memory/bin/
        if is_npx_bin_dir(exe_dir, &self.data_dir).await {
            let stamp_tmp = exe_dir.join(".version.ox1-tmp");
            let stamp_path = exe_dir.join(".version");
            tokio::fs::write(&stamp_tmp, format!("{}\n", target.version)).await?;
            tokio::fs::rename(&stamp_tmp, &stamp_path).await?;
        }

        Ok(())
    }

    async fn spawn_updated_daemon(&self) -> Result<()> {
        let log_file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .await
            .with_context(|| format!("Failed to open log file {}", self.log_path.display()))?
            .into_std()
            .await;
        let stderr_file = log_file
            .try_clone()
            .context("Failed to clone daemon log handle")?;

        let mut cmd = Command::new(&self.exe_path);
        cmd.args([
            "serve",
            "--foreground",
            "--port",
            &self.port.to_string(),
            "--wait-port-ms",
            "10000",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(stderr_file));

        #[cfg(unix)]
        cmd.process_group(0);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
        }

        let _child = cmd.spawn().with_context(|| {
            format!(
                "Failed to spawn updated Daemon from {}",
                self.exe_path.display()
            )
        })?;
        Ok(())
    }

    async fn set_runtime_stage(&self, stage: &str, target_version: Option<&str>) {
        {
            let mut rt = self.runtime.lock().await;
            rt.in_progress = stage != "idle" && stage != "error";
            rt.stage = stage.to_string();
            if let Some(v) = target_version {
                rt.target_version = Some(v.to_string());
            }
            rt.error = None;
        }
        let status = self.status().await;
        self.emit_status_event(&status);
    }

    async fn set_runtime_error(&self, target_version: &str, err: String) {
        {
            let mut rt = self.runtime.lock().await;
            rt.in_progress = false;
            rt.stage = "error".into();
            rt.target_version = Some(target_version.to_string());
            rt.error = Some(err);
        }
        let status = self.status().await;
        self.emit_status_event(&status);
    }

    fn emit_status_event(&self, status: &UpdateStatusResponse) {
        let _ = self.events_tx.send(StreamEvent {
            kind: "update_status".into(),
            timestamp: Utc::now().to_rfc3339(),
            payload: serde_json::to_value(status).unwrap_or_default(),
        });
    }

    pub fn spawn_background_checker(self: Arc<Self>, mut shutdown_rx: watch::Receiver<bool>) {
        tokio::spawn(async move {
            self.cleanup_old_binary_on_startup().await;

            // Brief startup delay so hooks/UI responsiveness isn't impacted at boot.
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                _ = wait_for_shutdown(&mut shutdown_rx) => return,
            }

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }
                let st = self.check_for_updates(false).await;
                if st.whats_new.should_show && st.whats_new.body.is_none() && st.auto_check_enabled
                {
                    let _ = self.fetch_whats_new_for_current().await;
                }

                let sleep_secs = self.next_check_delay_secs();
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(sleep_secs)) => {}
                    _ = wait_for_shutdown(&mut shutdown_rx) => break,
                }
            }
        });
    }

    fn next_check_delay_secs(&self) -> u64 {
        if let Ok(Some(last)) = self.db.get_meta(META_LAST_CHECKED_AT)
            && let Ok(ts) = DateTime::parse_from_rfc3339(&last)
        {
            let elapsed = (Utc::now() - ts.with_timezone(&Utc)).num_seconds().max(0);
            let remaining =
                (UPDATE_CHECK_INTERVAL_SECS - elapsed).clamp(60, UPDATE_CHECK_INTERVAL_SECS);
            return remaining as u64;
        }
        UPDATE_CHECK_INTERVAL_SECS as u64
    }
}

async fn wait_for_shutdown(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow() {
        if rx.changed().await.is_err() {
            break;
        }
    }
}

fn old_binary_path(exe_path: &Path) -> PathBuf {
    let mut os = exe_path.as_os_str().to_os_string();
    os.push(".old");
    PathBuf::from(os)
}

async fn replace_binary_file(staged: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        tokio::fs::rename(staged, target)
            .await
            .with_context(|| format!("Failed to replace binary {}", target.display()))?;
        Ok(())
    }
    #[cfg(windows)]
    {
        let old = old_binary_path(target);
        match tokio::fs::remove_file(&old).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Failed to remove previous .old binary"),
        }
        if tokio::fs::try_exists(target).await.unwrap_or(false) {
            tokio::fs::rename(target, &old)
                .await
                .with_context(|| format!("Failed to move running binary to {}", old.display()))?;
        }
        if let Err(err) = tokio::fs::rename(staged, target).await {
            if tokio::fs::try_exists(&old).await.unwrap_or(false) {
                let _ = tokio::fs::rename(&old, target).await;
            }
            return Err(err)
                .with_context(|| format!("Failed to move new binary to {}", target.display()));
        }
        Ok(())
    }
}

async fn is_npx_bin_dir(exe_dir: &Path, data_dir: &Path) -> bool {
    let npx_dir = data_dir.join("bin");
    if exe_dir == npx_dir {
        return true;
    }
    match (
        tokio::fs::canonicalize(exe_dir).await,
        tokio::fs::canonicalize(&npx_dir).await,
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

async fn resolve_system_bin_async(candidates: &[&str]) -> Result<PathBuf> {
    for candidate in candidates {
        let p = PathBuf::from(candidate);
        if tokio::fs::try_exists(&p).await.unwrap_or(false) {
            return Ok(p);
        }
    }
    bail!(
        "Required archive extraction tool not found in standard system paths: {}",
        candidates.join(", ")
    );
}

async fn extract_archive_async(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let name = archive_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if name.ends_with(".tar.gz") {
        let tar_bin = resolve_system_bin_async(&["/usr/bin/tar", "/bin/tar"]).await?;
        let out = Command::new(tar_bin)
            .arg("-xzf")
            .arg(archive_path)
            .arg("-C")
            .arg(dest_dir)
            .output()
            .await
            .context("Failed to spawn tar to extract update archive")?;
        if !out.status.success() {
            bail!(
                "tar extraction failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        return Ok(());
    }

    #[cfg(windows)]
    {
        let sys_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let ps_path = PathBuf::from(sys_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let ps_str = ps_path.to_string_lossy().into_owned();
        let ps_bin = resolve_system_bin_async(&[&ps_str]).await?;
        let arc_esc = archive_path.to_string_lossy().replace('\'', "''");
        let dst_esc = dest_dir.to_string_lossy().replace('\'', "''");
        let out = Command::new(ps_bin)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "Expand-Archive -Force -LiteralPath '{arc_esc}' -DestinationPath '{dst_esc}'"
                ),
            ])
            .output()
            .await
            .context("Failed to spawn PowerShell to extract zip update")?;
        if !out.status.success() {
            bail!(
                "PowerShell Expand-Archive failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        let unzip_bin = resolve_system_bin_async(&["/usr/bin/unzip", "/bin/unzip"]).await?;
        let out = Command::new(unzip_bin)
            .arg("-o")
            .arg(archive_path)
            .arg("-d")
            .arg(dest_dir)
            .output()
            .await
            .context("Failed to spawn unzip")?;
        if !out.status.success() {
            bail!(
                "unzip extraction failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

fn build_http_client(trusted_base: &str, timeout: Duration) -> Result<reqwest::Client> {
    let allow_local = trusted_base.starts_with("http://127.0.0.1");
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("Too many redirects while downloading update");
        }
        let url = attempt.url();
        let host = url.host_str().unwrap_or("").to_string();
        let allowed = if allow_local {
            host == "127.0.0.1" || host == "localhost"
        } else {
            url.scheme() == "https"
                && (host == "github.com"
                    || host == "api.github.com"
                    || host.ends_with(".github.com")
                    || host.ends_with(".githubusercontent.com"))
        };
        if allowed {
            attempt.follow()
        } else {
            attempt.error(format!("Blocked redirect to untrusted host: {host}"))
        }
    });

    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(policy)
        .build()
        .map_err(Into::into)
}

pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64, bool)> {
    let trimmed = raw.trim().trim_start_matches('v');
    if trimmed.is_empty() {
        return None;
    }
    let (core, pre) = match trimmed.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (trimmed, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, pre.is_some()))
}

pub fn is_newer_stable_release(current: &str, candidate: &str) -> bool {
    let Some((cur_maj, cur_min, cur_patch, cur_pre)) = parse_semver(current) else {
        return false;
    };
    let Some((cand_maj, cand_min, cand_patch, cand_pre)) = parse_semver(candidate) else {
        return false;
    };
    if cand_pre {
        return false;
    }
    (cand_maj, cand_min, cand_patch) > (cur_maj, cur_min, cur_patch)
        || ((cand_maj, cand_min, cand_patch) == (cur_maj, cur_min, cur_patch) && cur_pre)
}

pub fn is_newer_version(previous: &str, current: &str) -> bool {
    let Some((p_maj, p_min, p_patch, p_pre)) = parse_semver(previous) else {
        return false;
    };
    let Some((c_maj, c_min, c_patch, c_pre)) = parse_semver(current) else {
        return false;
    };
    (c_maj, c_min, c_patch, !c_pre) > (p_maj, p_min, p_patch, !p_pre)
}

pub fn platform_asset_name() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("0x1-memory-darwin-arm64.tar.gz")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("0x1-memory-linux-x64.tar.gz")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("0x1-memory-windows-x64.zip")
    } else {
        None
    }
}

pub fn validate_release_download_url(url: &str, trusted_base: &str) -> Result<()> {
    let base = trusted_base.trim_end_matches('/');
    let prefix = format!("{base}/v");
    if !url.starts_with(&prefix) {
        bail!("Download URL must originate from {prefix}*");
    }
    if url.contains("..")
        || url.contains('?')
        || url.contains('#')
        || url.contains('\\')
        || url.chars().any(char::is_whitespace)
    {
        bail!("Download URL contains unsafe path or query characters");
    }
    Ok(())
}

pub fn parse_sha256sums(sums_text: &str, asset: &str) -> Option<String> {
    for line in sums_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        let Some(file_raw) = parts.next() else {
            continue;
        };
        let file = file_raw.trim_start_matches('*');
        if file == asset && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn default_manual_command(install_kind: &str) -> String {
    match install_kind {
        "npx" => "npx -y 0x1-ai-memory@latest restart".into(),
        "dev_build" => "git pull && (cd ui && pnpm install && pnpm build) && cargo build --release".into(),
        "unsupported_platform" => {
            "git clone https://github.com/raihan0x1/0x1-ai-memory.git && cd 0x1-ai-memory && (cd ui && pnpm install && pnpm build) && cargo build --release".into()
        }
        _ if cfg!(windows) => "npx -y 0x1-ai-memory@latest restart".into(),
        _ => "curl -fsSL --proto '=https' --tlsv1.2 https://raw.githubusercontent.com/raihan0x1/0x1-ai-memory/main/install.sh | sh && 0x1-memory restart".into(),
    }
}

pub fn detect_capability_for(
    exe_path: &Path,
    data_dir: &Path,
    asset_name: Option<&str>,
) -> SelfUpdateCapability {
    let normalized = exe_path.to_string_lossy().replace('\\', "/");
    if normalized.contains("/target/debug/") || normalized.contains("/target/release/") {
        return SelfUpdateCapability {
            supported: false,
            reason: Some(
                "Running from a local Cargo dev/release build. Rebuild from source or install a release binary to self-update."
                    .into(),
            ),
            install_kind: "dev_build".into(),
            manual_command: default_manual_command("dev_build"),
            asset_name: asset_name.map(ToOwned::to_owned),
        };
    }

    let Some(asset) = asset_name else {
        return SelfUpdateCapability {
            supported: false,
            reason: Some("No prebuilt binary is published for this OS/architecture.".into()),
            install_kind: "unsupported_platform".into(),
            manual_command: default_manual_command("unsupported_platform"),
            asset_name: None,
        };
    };

    let Some(parent) = exe_path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return SelfUpdateCapability {
            supported: false,
            reason: Some("Could not determine binary install directory.".into()),
            install_kind: "binary".into(),
            manual_command: default_manual_command("binary"),
            asset_name: Some(asset.to_string()),
        };
    };

    let npx_dir = data_dir.join("bin");
    let is_npx = match (
        std::fs::canonicalize(parent),
        std::fs::canonicalize(&npx_dir),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => parent == npx_dir,
    };
    let install_kind = if is_npx { "npx" } else { "binary" };
    let manual_command = default_manual_command(install_kind);

    let probe = parent.join(format!(".ox1-write-probe-{}", std::process::id()));
    let dir_writable = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map(|_| {
            let _ = std::fs::remove_file(&probe);
            true
        })
        .unwrap_or(false);

    #[cfg(windows)]
    let file_readonly = exe_path.exists()
        && std::fs::metadata(exe_path)
            .map(|m| m.permissions().readonly())
            .unwrap_or(false);
    #[cfg(not(windows))]
    let file_readonly = false;

    if !dir_writable || file_readonly {
        return SelfUpdateCapability {
            supported: false,
            reason: Some(format!(
                "Install location {} is read-only for the current user.",
                parent.display()
            )),
            install_kind: install_kind.into(),
            manual_command,
            asset_name: Some(asset.to_string()),
        };
    }

    SelfUpdateCapability {
        supported: true,
        reason: None,
        install_kind: install_kind.into(),
        manual_command,
        asset_name: Some(asset.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn semver_comparison_ignores_prereleases_for_stable_updates() {
        assert!(is_newer_stable_release("0.3.1", "v0.3.2"));
        assert!(is_newer_stable_release("0.3.1", "0.4.0"));
        assert!(!is_newer_stable_release("0.3.1", "0.3.1"));
        assert!(!is_newer_stable_release("0.3.1", "0.3.0"));
        assert!(!is_newer_stable_release("0.3.1", "v0.3.2-rc.1"));
        assert!(is_newer_stable_release("0.3.2-rc.1", "v0.3.2"));

        assert!(is_newer_version("0.0.0", "0.3.1"));
        assert!(is_newer_version("0.3.0", "0.3.2-rc.1"));
        assert!(!is_newer_version("0.3.1", "0.3.1"));
    }

    #[test]
    fn validates_pinned_release_urls_and_parses_sha256sums() {
        assert!(
            validate_release_download_url(
                "https://github.com/raihan0x1/0x1-ai-memory/releases/download/v0.3.2/0x1-memory-darwin-arm64.tar.gz",
                DEFAULT_GITHUB_RELEASES_BASE
            )
            .is_ok()
        );
        assert!(
            validate_release_download_url(
                "https://evil.example.com/v0.3.2/0x1-memory-darwin-arm64.tar.gz",
                DEFAULT_GITHUB_RELEASES_BASE
            )
            .is_err()
        );
        assert!(
            validate_release_download_url(
                "https://github.com/raihan0x1/0x1-ai-memory/releases/download/v0.3.2/../evil.tar.gz",
                DEFAULT_GITHUB_RELEASES_BASE
            )
            .is_err()
        );

        let sums = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  0x1-memory-linux-x64.tar.gz\nBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB *0x1-memory-darwin-arm64.tar.gz\n";
        assert_eq!(
            parse_sha256sums(sums, "0x1-memory-darwin-arm64.tar.gz").as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        assert!(parse_sha256sums(sums, "0x1-memory-windows-x64.zip").is_none());
    }

    #[test]
    fn capability_detects_dev_builds_npx_dir_and_unsupported_platforms() {
        let tmp = tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let npx_bin_dir = data_dir.join("bin");
        std::fs::create_dir_all(&npx_bin_dir).unwrap();

        let dev = detect_capability_for(
            Path::new("/Users/dev/0x1-ai-memory/target/debug/0x1-memory"),
            &data_dir,
            Some("0x1-memory-darwin-arm64.tar.gz"),
        );
        assert!(!dev.supported);
        assert_eq!(dev.install_kind, "dev_build");

        let unsup = detect_capability_for(&npx_bin_dir.join("0x1-memory"), &data_dir, None);
        assert!(!unsup.supported);
        assert_eq!(unsup.install_kind, "unsupported_platform");

        let npx = detect_capability_for(
            &npx_bin_dir.join("0x1-memory"),
            &data_dir,
            Some("0x1-memory-darwin-arm64.tar.gz"),
        );
        assert!(npx.supported);
        assert_eq!(npx.install_kind, "npx");
        assert!(npx.manual_command.contains("npx"));
    }
}
