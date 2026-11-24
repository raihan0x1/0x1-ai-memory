use crate::config::{EMBEDDING_DIM, Settings};
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
    http: reqwest::Client,
}

impl Embedder {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            models_dir,
            local_model: Arc::new(Mutex::new(None)),
            local_state: Arc::new(Mutex::new(LocalModelState::Idle)),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Triggers background download/load of the quantized 384-dim ONNX model without blocking startup.
    pub fn warm_local_async(&self) {
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

    /// Computes a 384-dim embedding if ready/configured. Never blocks startup or crashes FTS5 search.
    pub async fn embed_text(&self, text: &str, settings: &Settings) -> Option<Vec<f32>> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        match settings.embedding_mode.as_str() {
            "fts_only" => None,
            "api" => match self.embed_via_api(trimmed, settings).await {
                Ok(vec) => Some(vec),
                Err(e) => {
                    tracing::warn!(error = %e, "API embedding failed; falling back to FTS5");
                    None
                }
            },
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

    async fn embed_via_api(&self, text: &str, settings: &Settings) -> Result<Vec<f32>> {
        let base = settings.effective_base_url();
        let url = format!("{base}/embeddings");
        let model = if settings.embedding_model.trim().is_empty() {
            "text-embedding-3-small".to_string()
        } else {
            settings.embedding_model.clone()
        };
        let mut body = json!({
            "model": model,
            "input": text.chars().take(4000).collect::<String>(),
        });
        if model.contains("text-embedding-3") {
            body["dimensions"] = json!(EMBEDDING_DIM);
        }
        let mut req = self.http.post(&url).json(&body);
        if let Some(key) = settings.resolve_api_key() {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.context("Embedding HTTP request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Embedding API {status}: {err_body}");
        }
        let val: serde_json::Value = resp.json().await?;
        let arr = val["data"][0]["embedding"]
            .as_array()
            .context("Missing data[0].embedding in response")?;
        let raw: Vec<f32> = arr
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();
        Ok(fit_to_dim(&raw, EMBEDDING_DIM))
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
