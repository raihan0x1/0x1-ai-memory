use crate::db::MemoryDb;
use crate::embed::Embedder;
use crate::llm::MemoryLlmClient;
use crate::models::{NewMemoryInput, SearchFilter, StreamEvent};
use chrono::Utc;
use serde_json::json;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

#[derive(Clone)]
pub struct DistillHandle {
    tx: mpsc::UnboundedSender<String>,
}

impl DistillHandle {
    pub fn queue_session(&self, session_id: String) {
        let _ = self.tx.send(session_id);
    }
}

pub fn spawn_distill_worker(
    db: MemoryDb,
    embedder: Embedder,
    llm: MemoryLlmClient,
    events_tx: broadcast::Sender<StreamEvent>,
) -> DistillHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    tokio::spawn(async move {
        let mut sweep_interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                Some(session_id) = rx.recv() => {
                    distill_session_once(&db, &embedder, &llm, &events_tx, &session_id).await;
                }
                _ = sweep_interval.tick() => {
                    run_periodic_maintenance(&db, &embedder, &llm, &events_tx).await;
                }
            }
        }
    });

    DistillHandle { tx }
}

pub async fn distill_session_once(
    db: &MemoryDb,
    embedder: &Embedder,
    llm: &MemoryLlmClient,
    events_tx: &broadcast::Sender<StreamEvent>,
    session_id: &str,
) {
    let Ok(events) = db.pending_events_for_session(session_id, 40) else {
        return;
    };
    if events.is_empty() {
        return;
    }

    let project_id = events[0].project_id.clone();
    let source_client = events[0].source_client.clone();
    let event_ids: Vec<String> = events.iter().map(|e| e.id.clone()).collect();

    let Ok(settings) = db.get_settings() else {
        return;
    };

    // Respect per-project privacy mode before touching any external LLM API
    let project = db.get_project(&project_id).ok().flatten();
    let project_name = project
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| project_id.clone());
    let privacy_mode = project
        .as_ref()
        .map(|p| p.privacy_mode.as_str())
        .unwrap_or("full");

    if privacy_mode == "paused" {
        let _ = db.mark_events_distilled(&event_ids);
        return;
    }

    if settings.provider == "none" {
        // Zero-LLM safe mode: keep capture overhead at 0 tokens; user uses memory_save or enables a provider later.
        let _ = db.mark_events_distilled(&event_ids);
        return;
    }

    if privacy_mode == "local_only" && settings.is_external_destination() {
        tracing::info!(
            project = %project_name,
            provider = %settings.provider,
            "Skipping off-machine LLM distillation for local_only project"
        );
        let _ = db.mark_events_distilled(&event_ids);
        return;
    }

    // Hard budget guardrail check BEFORE calling LLM
    let emb_status = embedder.status_label(&settings);
    if let Ok(summary) = db.usage_summary(&settings, emb_status)
        && summary.budget_exhausted
    {
        let _ = db.record_usage(
            &settings.provider,
            &settings.model,
            "distill",
            0,
            0,
            0.0,
            Some(session_id),
            "budget_blocked",
            Some("Daily token or USD budget cap reached"),
        );
        let _ = db.mark_events_distilled(&event_ids);
        let _ = events_tx.send(StreamEvent {
            kind: "usage_recorded".into(),
            timestamp: Utc::now().to_rfc3339(),
            payload: json!({ "status": "budget_blocked", "session_id": session_id }),
        });
        return;
    }

    let existing_recent = db
        .search_memories(
            &SearchFilter {
                project_id: Some(project_id.clone()),
                limit: Some(6),
                ..Default::default()
            },
            None,
        )
        .unwrap_or_default();

    match llm
        .distill_turn(
            &settings,
            &project_name,
            &source_client,
            &events,
            &existing_recent,
        )
        .await
    {
        Ok(result) => {
            let _ = db.record_usage(
                &settings.provider,
                &settings.model,
                "distill",
                result.input_tokens,
                result.output_tokens,
                result.estimated_cost_usd,
                Some(session_id),
                "ok",
                None,
            );
            let _ = db.mark_events_distilled(&event_ids);

            for item in result.memories.into_iter().take(3) {
                if item.title.trim().is_empty() || item.summary.trim().is_empty() {
                    continue;
                }
                let embed_input = format!("{}\n{}\n{}", item.title, item.summary, item.content);
                let vec = embedder
                    .embed_text(
                        &embed_input,
                        &settings,
                        Some(db),
                        privacy_mode != "local_only",
                    )
                    .await;
                let input = NewMemoryInput {
                    scope: item.scope,
                    project_id: Some(project_id.clone()),
                    session_id: Some(session_id.to_string()),
                    source_client: Some(source_client.clone()),
                    memory_type: item.memory_type,
                    title: item.title,
                    summary: item.summary,
                    content: item.content,
                    files: Some(item.files),
                    tags: Some(item.tags),
                    status: Some("active".into()),
                    supersedes_id: item.supersedes_id,
                };
                if let Ok(mem) = db.create_memory(input, vec.as_deref()) {
                    let _ = events_tx.send(StreamEvent {
                        kind: "memory_created".into(),
                        timestamp: Utc::now().to_rfc3339(),
                        payload: serde_json::to_value(&mem).unwrap_or_default(),
                    });
                }
            }

            let _ = events_tx.send(StreamEvent {
                kind: "usage_recorded".into(),
                timestamp: Utc::now().to_rfc3339(),
                payload: json!({
                    "status": "ok",
                    "input_tokens": result.input_tokens,
                    "output_tokens": result.output_tokens,
                    "cost_usd": result.estimated_cost_usd,
                }),
            });
        }
        Err(e) => {
            // Strict zero-silent-fallback: record error and mark events so we don't spin-loop burn.
            let msg = e.to_string();
            tracing::warn!(session_id, error = %msg, "Distillation failed (zero silent fallback)");
            let _ = db.record_usage(
                &settings.provider,
                &settings.model,
                "distill",
                0,
                0,
                0.0,
                Some(session_id),
                "error",
                Some(&msg),
            );
            let _ = db.mark_events_distilled(&event_ids);
            let _ = events_tx.send(StreamEvent {
                kind: "usage_recorded".into(),
                timestamp: Utc::now().to_rfc3339(),
                payload: json!({
                    "status": "error",
                    "session_id": session_id,
                    "provider": settings.provider,
                    "model": settings.model,
                    "error_message": msg,
                }),
            });
        }
    }
}

async fn run_periodic_maintenance(
    db: &MemoryDb,
    embedder: &Embedder,
    llm: &MemoryLlmClient,
    events_tx: &broadcast::Sender<StreamEvent>,
) {
    let Ok(settings) = db.get_settings() else {
        return;
    };

    // 1. Idle / crash-recovery sweep for sessions that didn't emit Stop
    if let Ok(idle_sessions) = db.sessions_with_idle_pending_events(settings.distill_idle_seconds) {
        for sid in idle_sessions {
            distill_session_once(db, embedder, llm, events_tx, &sid).await;
        }
    }

    // 2. Backfill embeddings once lazy ONNX embedder becomes ready (respecting local_only)
    if settings.embedding_mode != "fts_only"
        && let Ok(missing) = db.memories_missing_embeddings(10)
    {
        for (mem_id, proj_id, text) in missing {
            let allow_ext = !db.is_project_local_only(proj_id.as_deref());
            if let Some(vec) = embedder
                .embed_text(&text, &settings, Some(db), allow_ext)
                .await
            {
                let _ = db.set_memory_embedding(&mem_id, &vec);
            } else if settings.embedding_mode == "local_onnx" {
                break;
            }
        }
    }

    // 3. Retention pruning for old raw_events
    let _ = db.prune_old_raw_events(settings.raw_event_retention_days);
}
