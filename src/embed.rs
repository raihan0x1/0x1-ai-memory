use crate::config::{EMBEDDING_DIM, ProviderHttp, Settings};
use crate::db::MemoryDb;
use crate::scrub::redact_secrets;
use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub enum LocalModelState {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

/// Pluggable embedder supporting lazy local ONNX (zero Python/Chroma), API embeddings, or FTS-only.
#[derive(Clone)]
pub struct Embedder {
    models_dir: PathBuf,
    local_model: Arc<Mutex<Option<TextEmbedding>>>,
    local_state: Arc<Mutex<LocalModelState>>,
    http: ProviderHttp,
}

impl Embedder {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            models_dir,
            local_model: Arc::new(Mutex::new(None)),
            local_state: Arc::new(Mutex::new(LocalModelState::Idle)),
            http: ProviderHttp::new(std::time::Duration::from_secs(15)),
        }
    }

    /// Triggers background download/load of the quantized 384-dim ONNX model without blocking startup.
    pub fn warm_local_async(&self) {
        if cfg!(test) || std::env::var_os("OX1_MEMORY_DISABLE_ONNX").is_some() {
            let mut state = self.local_state.lock().unwrap();
            if matches!(*state, LocalModelState::Idle) {
                *state = LocalModelState::Failed("disabled (offline/test mode)".into());
            }
            return;
        }
        let should_start = {
            let mut state = self.local_state.lock().unwrap();
            match *state {
                LocalModelState::Idle => {
                    *state = LocalModelState::Loading;
                    true
                }
                _ => false,
            }
        };
        if !should_start {
            return;
        }
        let models_dir = self.models_dir.clone();
        let local_model = self.local_model.clone();
        let local_state = self.local_state.clone();

        tokio::task::spawn_blocking(move || {
            let _ = std::fs::create_dir_all(&models_dir);
            let opts = TextInitOptions::new(EmbeddingModel::AllMiniLML6V2Q)
                .with_cache_dir(models_dir)
                .with_show_download_progress(false);
            match TextEmbedding::try_new(opts) {
                Ok(model) => {
                    *local_model.lock().unwrap() = Some(model);
                    *local_state.lock().unwrap() = LocalModelState::Ready;
                    tracing::info!("Local ONNX embedder ready (AllMiniLML6V2Q, 384-dim)");
                }
                Err(e) => {
                    let msg = e.to_string();
                    tracing::warn!(error = %msg, "Local ONNX embedder unavailable; falling back to FTS5");
                    *local_state.lock().unwrap() = LocalModelState::Failed(msg);
                }
            }
        });
    }

    pub fn status_label(&self, settings: &Settings) -> String {
        match settings.embedding_mode.as_str() {
            "fts_only" => "fts_only".into(),
            "api" => format!("api ({})", settings.embedding_model),
            _ => match &*self.local_state.lock().unwrap() {
                LocalModelState::Idle => "idle (lazy local_onnx)".into(),
                LocalModelState::Loading => "loading local_onnx (~25MB)...".into(),
                LocalModelState::Ready => "ready (local_onnx 384d)".into(),
                LocalModelState::Failed(e) => format!("fts5 fallback ({})", truncate_err(e)),
            },
        }
    }

    /// Computes a 384-dim embedding if ready/configured.
    /// Always redacts secrets FIRST, respects `local_only` privacy, and enforces daily caps + ledger on API calls.
    pub async fn embed_text(
        &self,
        text: &str,
        settings: &Settings,
        db: Option<&MemoryDb>,
        allow_external_api: bool,
    ) -> Option<Vec<f32>> {
        let scrubbed = redact_secrets(text);
        let trimmed = scrubbed.trim();
        if trimmed.is_empty() {
            return None;
        }
        match settings.embedding_mode.as_str() {
            "fts_only" => None,
            "api" => {
                if !allow_external_api && !settings.is_local_base_url() {
                    tracing::info!("Skipping external API embedding for local_only project");
                    return None;
                }
                let ledger_db = db?;
                let summary = match ledger_db.usage_summary(settings, self.status_label(settings)) {
                    Ok(summary) => summary,
                    Err(error) => {
                        tracing::warn!(%error, "API embedding skipped because usage could not be read");
                        return None;
                    }
                };
                if summary.budget_exhausted {
                    let _ = ledger_db.record_usage(
                        &settings.provider,
                        &settings.embedding_model,
                        "embed",
                        0,
                        0,
                        0.0,
                        None,
                        "budget_blocked",
                        Some("Daily budget cap reached before API embedding"),
                    );
                    return None;
                }
                match self.embed_via_api(trimmed, settings).await {
                    Ok((vec, tokens)) => {
                        let cost = if settings.is_local_base_url() {
                            0.0
                        } else {
                            (tokens as f64 / 1_000_000.0) * 0.02
                        };
                        let _ = ledger_db.record_usage(
                            &settings.provider,
                            &settings.embedding_model,
                            "embed",
                            tokens,
                            0,
                            cost,
                            None,
                            "ok",
                            None,
                        );
                        Some(vec)
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let _ = ledger_db.record_usage(
                            &settings.provider,
                            &settings.embedding_model,
                            "embed",
                            0,
                            0,
                            0.0,
                            None,
                            "error",
                            Some(&msg),
                        );
                        tracing::warn!(error = %msg, "API embedding failed; falling back to FTS5");
                        None
                    }
                }
            }
            _ => {
                let state = self.local_state.lock().unwrap().clone();
                match state {
                    LocalModelState::Idle => {
                        self.warm_local_async();
                        None
                    }
                    LocalModelState::Loading | LocalModelState::Failed(_) => None,
                    LocalModelState::Ready => {
                        let model_arc = self.local_model.clone();
                        let input = trimmed.chars().take(2000).collect::<String>();
                        tokio::task::spawn_blocking(move || {
                            let mut guard = model_arc.lock().unwrap();
                            let model = guard.as_mut()?;
                            let mut res = model.embed(vec![input], None).ok()?;
                            let vec = res.pop()?;
                            if vec.len() == EMBEDDING_DIM {
                                Some(vec)
                            } else {
                                None
                            }
                        })
                        .await
                        .ok()
                        .flatten()
                    }
                }
            }
        }
    }

    async fn embed_via_api(&self, text: &str, settings: &Settings) -> Result<(Vec<f32>, i64)> {
        let base = settings.effective_base_url();
        let input_text = text.chars().take(4000).collect::<String>();
        let est_tokens = (input_text.len() / 4).max(1) as i64;

        if settings.provider == "gemini" {
            let api_key = settings
                .resolve_api_key()
                .context("Missing Gemini API key for embeddings")?;
            let model = if settings.embedding_model.trim().is_empty()
                || settings.embedding_model == "all-MiniLM-L6-v2"
            {
                "text-embedding-004".to_string()
            } else {
                settings
                    .embedding_model
                    .trim_start_matches("models/")
                    .to_string()
            };
            let url = format!("{base}/models/{model}:embedContent");
            let body = json!({
                "model": format!("models/{model}"),
                "content": { "parts": [{ "text": input_text }] },
                "outputDimensionality": EMBEDDING_DIM
            });
            let resp = self
                .http
                .for_settings(settings)
                .post(&url)
                .header("x-goog-api-key", api_key)
                .json(&body)
                .send()
                .await
                .context("Gemini embedding HTTP request failed")?;
            if !resp.status().is_success() {
                let status = resp.status();
                anyhow::bail!("Gemini Embedding API returned HTTP {status}");
            }
            let val: serde_json::Value = resp.json().await?;
            let arr = val["embedding"]["values"]
                .as_array()
                .context("Missing embedding.values in Gemini response")?;
            let raw: Vec<f32> = arr
                .iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect();
            return Ok((fit_to_dim(&raw, EMBEDDING_DIM), est_tokens));
        }

        let url = format!("{base}/embeddings");
        let model = if settings.embedding_model.trim().is_empty()
            || settings.embedding_model == "all-MiniLM-L6-v2"
        {
            "text-embedding-3-small".to_string()
        } else {
            settings.embedding_model.clone()
        };
        let mut body = json!({
            "model": model,
            "input": input_text,
        });
        if model.contains("text-embedding-3") {
            body["dimensions"] = json!(EMBEDDING_DIM);
        }
        let mut req = self.http.for_settings(settings).post(&url).json(&body);
        if let Some(key) = settings.resolve_api_key() {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.context("Embedding HTTP request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            anyhow::bail!("Embedding API returned HTTP {status}");
        }
        let val: serde_json::Value = resp.json().await?;
        let arr = val["data"][0]["embedding"]
            .as_array()
            .context("Missing data[0].embedding in response")?;
        let tokens = val["usage"]["total_tokens"]
            .as_i64()
            .filter(|tokens| *tokens >= 0)
            .unwrap_or(est_tokens);
        let raw: Vec<f32> = arr
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();
        Ok((fit_to_dim(&raw, EMBEDDING_DIM), tokens))
    }
}

/// Fits an arbitrary vector to 384 dims via Matryoshka slice/zero-pad + L2 normalization.
fn fit_to_dim(raw: &[f32], dim: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; dim];
    let n = raw.len().min(dim);
    out[..n].copy_from_slice(&raw[..n]);
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        for x in &mut out {
            *x /= norm;
        }
    }
    out
}

fn truncate_err(s: &str) -> String {
    s.chars().take(60).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_local_async_stays_offline_in_tests() {
        let tmp = tempfile::tempdir().unwrap();
        let embedder = Embedder::new(tmp.path().join("models"));
        embedder.warm_local_async();
        assert!(matches!(
            *embedder.local_state.lock().unwrap(),
            LocalModelState::Failed(_)
        ));
    }
}
