use crate::config::Settings;
use crate::models::{MemoryHit, RawEvent};
use crate::scrub::{redact_secrets, truncate_head_tail};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    pub scope: Option<String>,
    #[serde(rename = "type")]
    pub memory_type: String,
    pub title: String,
    pub summary: String,
    pub content: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub supersedes_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtractionEnvelope {
    #[serde(default)]
    pub memories: Vec<ExtractedMemory>,
}

#[derive(Debug, Clone)]
pub struct LlmCallResult {
    pub memories: Vec<ExtractedMemory>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionTestResult {
    pub ok: bool,
    pub provider: String,
    pub model: String,
    pub latency_ms: u64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub message: String,
}

#[derive(Clone)]
pub struct MemoryLlmClient {
    http: reqwest::Client,
}

impl Default for MemoryLlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryLlmClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(45))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Stateless per-turn distillation. Strictly respects chosen provider with ZERO silent fallback.
    pub async fn distill_turn(
        &self,
        settings: &Settings,
        project_name: &str,
        source_client: &str,
        events: &[RawEvent],
        existing_recent: &[MemoryHit],
    ) -> Result<LlmCallResult> {
        if settings.provider == "none" {
            bail!("Zero-LLM mode is active (provider = none)");
        }
        let (system_prompt, user_prompt) =
            build_distill_prompts(project_name, source_client, events, existing_recent);

        let raw = self
            .complete_json(settings, &system_prompt, &user_prompt)
            .await?;

        let envelope = parse_extraction_json(&raw.text)?;
        Ok(LlmCallResult {
            memories: envelope.memories,
            input_tokens: raw.input_tokens,
            output_tokens: raw.output_tokens,
            estimated_cost_usd: raw.cost_usd,
        })
    }

    /// Minimal connectivity verification call for the Web UI & CLI Setup Wizard.
    pub async fn test_connection(&self, settings: &Settings) -> ConnectionTestResult {
        let start = std::time::Instant::now();
        if settings.provider == "none" {
            return ConnectionTestResult {
                ok: true,
                provider: "none".into(),
                model: "zero-llm".into(),
                latency_ms: 0,
                input_tokens: 0,
                output_tokens: 0,
                message: "Zero-LLM mode active. FTS5/vector search and explicit memory_save work at $0 cost.".into(),
            };
        }
        match self
            .complete_json(
                settings,
                "Reply strictly with JSON: {\"ok\": true}",
                "Ping check. Reply {\"ok\": true}.",
            )
            .await
        {
            Ok(res) => ConnectionTestResult {
                ok: true,
                provider: settings.provider.clone(),
                model: settings.model.clone(),
                latency_ms: start.elapsed().as_millis() as u64,
                input_tokens: res.input_tokens,
                output_tokens: res.output_tokens,
                message: format!(
                    "Connected to {} ({}) in {}ms",
                    settings.provider,
                    settings.model,
                    start.elapsed().as_millis()
                ),
            },
            Err(e) => ConnectionTestResult {
                ok: false,
                provider: settings.provider.clone(),
                model: settings.model.clone(),
                latency_ms: start.elapsed().as_millis() as u64,
                input_tokens: 0,
                output_tokens: 0,
                message: format!("Connection failed (no silent fallback used): {e}"),
            },
        }
    }

    async fn complete_json(
        &self,
        settings: &Settings,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<RawCompletion> {
        // Guardrail: exact dispatch by configured provider; NEVER silently fall back to coding subscriptions.
        match settings.provider.as_str() {
            "openrouter" => {
                self.call_openai_compat(settings, system_prompt, user_prompt, true)
                    .await
            }
            "openai_compatible" | "ollama" => {
                self.call_openai_compat(settings, system_prompt, user_prompt, false)
                    .await
            }
            "gemini" => self.call_gemini(settings, system_prompt, user_prompt).await,
            "cli_claude" => {
                self.call_cli_claude(settings, system_prompt, user_prompt)
                    .await
            }
            "cli_codex" => {
                self.call_cli_codex(settings, system_prompt, user_prompt)
                    .await
            }
            "cli_agy" => {
                self.call_cli_agy(settings, system_prompt, user_prompt)
                    .await
            }
            other => bail!("Unsupported provider: {other}"),
        }
    }

    async fn call_openai_compat(
        &self,
        settings: &Settings,
        system_prompt: &str,
        user_prompt: &str,
        is_openrouter: bool,
    ) -> Result<RawCompletion> {
        let base = settings.effective_base_url();
        let url = format!("{base}/chat/completions");
        let api_key = settings.resolve_api_key();
        if is_openrouter && api_key.is_none() {
            bail!("Missing OpenRouter API key (set api_key or env:OPENROUTER_API_KEY)");
        }
        let model = if settings.model.trim().is_empty() {
            if settings.provider == "ollama" {
                "qwen2.5-coder:7b".to_string()
            } else {
                "google/gemini-2.5-flash-lite".to_string()
            }
        } else {
            settings.model.clone()
        };

        let mut body = json!({
            "model": model,
            "temperature": 0.1,
            "max_tokens": 900,
            "response_format": { "type": "json_object" },
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt }
            ]
        });
        if is_openrouter {
            // Avoid hidden reasoning token burn and request exact usage/cost metadata.
            body["reasoning"] = json!({ "enabled": false });
            body["usage"] = json!({ "include": true });
        }

        let mut req = self.http.post(&url).json(&body);
        if let Some(key) = api_key {
            req = req.bearer_auth(key);
        }
        if is_openrouter {
            req = req
                .header("HTTP-Referer", "https://github.com/0x1-ai-memory")
                .header("X-Title", "0x1-ai-memory");
        }

        let resp = req.send().await.context("HTTP LLM request failed")?;
        let status = resp.status();
        let resp_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "Provider {} returned HTTP {}: {}",
                settings.provider,
                status,
                truncate_head_tail(&resp_text, 400)
            );
        }

        let v: serde_json::Value =
            serde_json::from_str(&resp_text).context("Invalid JSON from LLM provider")?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let input_tokens = v["usage"]["prompt_tokens"].as_i64().unwrap_or(0);
        let output_tokens = v["usage"]["completion_tokens"].as_i64().unwrap_or(0);
        let reported_cost = v["usage"]["cost"].as_f64();
        let cost_usd = reported_cost.unwrap_or_else(|| {
            estimate_cost_usd(&settings.provider, &model, input_tokens, output_tokens)
        });

        Ok(RawCompletion {
            text,
            input_tokens,
            output_tokens,
            cost_usd,
        })
    }

    async fn call_gemini(
        &self,
        settings: &Settings,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<RawCompletion> {
        let api_key = settings
            .resolve_api_key()
            .context("Missing Gemini API key (set api_key or env:GEMINI_API_KEY)")?;
        let base = settings.effective_base_url();
        let model = if settings.model.trim().is_empty() {
            "gemini-2.5-flash-lite".to_string()
        } else {
            settings.model.trim_start_matches("google/").to_string()
        };
        let url = format!("{base}/models/{model}:generateContent");
        let body = json!({
            "systemInstruction": { "parts": [{ "text": system_prompt }] },
            "contents": [{ "role": "user", "parts": [{ "text": user_prompt }] }],
            "generationConfig": {
                "temperature": 0.1,
                "maxOutputTokens": 900,
                "responseMimeType": "application/json"
            }
        });

        let resp = self
            .http
            .post(&url)
            .header("x-goog-api-key", api_key)
            .json(&body)
            .send()
            .await
            .context("Gemini request failed")?;
        let status = resp.status();
        let resp_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "Gemini returned HTTP {}: {}",
                status,
                truncate_head_tail(&resp_text, 400)
            );
        }

        let v: serde_json::Value = serde_json::from_str(&resp_text)?;
        let text = v["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let input_tokens = v["usageMetadata"]["promptTokenCount"].as_i64().unwrap_or(0);
        let output_tokens = v["usageMetadata"]["candidatesTokenCount"]
            .as_i64()
            .unwrap_or(0);
        let cost_usd = estimate_cost_usd("gemini", &model, input_tokens, output_tokens);

        Ok(RawCompletion {
            text,
            input_tokens,
            output_tokens,
            cost_usd,
        })
    }

    /// Strictly opt-in Claude CLI passthrough with recursion guard and disabled hooks/MCP/persistence.
    async fn call_cli_claude(
        &self,
        settings: &Settings,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<RawCompletion> {
        let combined = format!("{system_prompt}\n\n{user_prompt}");
        let mut cmd = Command::new("claude");
        cmd.env("OX1_MEMORY_INTERNAL", "1")
            .arg("-p")
            .arg(&combined)
            .arg("--output-format")
            .arg("json")
            .arg("--no-session-persistence")
            .arg("--tools")
            .arg("")
            .arg("--strict-mcp-config")
            .arg("--settings")
            .arg(r#"{"disableAllHooks":true}"#);
        if !settings.model.trim().is_empty() && !settings.model.contains('/') {
            cmd.arg("--model").arg(&settings.model);
        }
        let out = tokio::time::timeout(Duration::from_secs(45), cmd.output())
            .await
            .context("claude -p timed out")?
            .context("Failed to spawn `claude` CLI")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("claude -p failed: {}", truncate_head_tail(&err, 300));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let v: serde_json::Value =
            serde_json::from_str(&stdout).unwrap_or_else(|_| json!({ "result": stdout }));
        let text = v["result"].as_str().unwrap_or(&stdout).to_string();
        let input_tokens = v["usage"]["input_tokens"]
            .as_i64()
            .unwrap_or((combined.len() / 4) as i64);
        let output_tokens = v["usage"]["output_tokens"]
            .as_i64()
            .unwrap_or((text.len() / 4) as i64);
        Ok(RawCompletion {
            text,
            input_tokens,
            output_tokens,
            cost_usd: 0.0,
        })
    }

    /// Strictly opt-in Codex CLI passthrough with recursion guard and disabled hooks.
    async fn call_cli_codex(
        &self,
        settings: &Settings,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<RawCompletion> {
        let combined = format!("{system_prompt}\n\n{user_prompt}");
        let mut cmd = Command::new("codex");
        cmd.env("OX1_MEMORY_INTERNAL", "1")
            .arg("exec")
            .arg(&combined)
            .arg("--ephemeral")
            .arg("--skip-git-repo-check")
            .arg("-s")
            .arg("read-only")
            .arg("-c")
            .arg("features.hooks=false")
            .arg("-c")
            .arg("features.codex_hooks=false");
        if !settings.model.trim().is_empty() && !settings.model.contains('/') {
            cmd.arg("-m").arg(&settings.model);
        }
        let out = tokio::time::timeout(Duration::from_secs(45), cmd.output())
            .await
            .context("codex exec timed out")?
            .context("Failed to spawn `codex` CLI")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("codex exec failed: {}", truncate_head_tail(&err, 300));
        }
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        Ok(RawCompletion {
            input_tokens: (combined.len() / 4) as i64,
            output_tokens: (text.len() / 4) as i64,
            text,
            cost_usd: 0.0,
        })
    }

    /// Strictly opt-in Antigravity CLI passthrough with OX1_MEMORY_INTERNAL=1 recursion guard.
    async fn call_cli_agy(
        &self,
        settings: &Settings,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<RawCompletion> {
        let combined = format!("{system_prompt}\n\n{user_prompt}");
        let mut cmd = Command::new("agy");
        cmd.env("OX1_MEMORY_INTERNAL", "1")
            .arg("-p")
            .arg(&combined)
            .arg("--output-format")
            .arg("json")
            .arg("--disable-slash-commands");
        if !settings.model.trim().is_empty() && !settings.model.contains('/') {
            cmd.arg("--model").arg(&settings.model);
        }
        let out = tokio::time::timeout(Duration::from_secs(45), cmd.output())
            .await
            .context("agy -p timed out")?
            .context("Failed to spawn `agy` CLI")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("agy -p failed: {}", truncate_head_tail(&err, 300));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let v: serde_json::Value =
            serde_json::from_str(&stdout).unwrap_or_else(|_| json!({ "result": stdout }));
        let text = v["result"].as_str().unwrap_or(&stdout).to_string();
        Ok(RawCompletion {
            input_tokens: (combined.len() / 4) as i64,
            output_tokens: (text.len() / 4) as i64,
            text,
            cost_usd: 0.0,
        })
    }
}

struct RawCompletion {
    text: String,
    input_tokens: i64,
    output_tokens: i64,
    cost_usd: f64,
}

fn build_distill_prompts(
    project_name: &str,
    source_client: &str,
    events: &[RawEvent],
    existing_recent: &[MemoryHit],
) -> (String, String) {
    let system = r#"You are 0x1-ai-memory's stateless per-turn memory distiller.
Extract ONLY durable, high-signal engineering knowledge from this completed coding turn so other AI agents (Claude Code, Codex, Antigravity, T3 Code) working on the same project stay aligned.

Rules:
1. Ignore routine exploration noise (mere file reads, trivial ls/grep, typos). If nothing durable was learned or decided, return {"memories":[]}.
2. Return at most 3 concise memories per turn.
3. Valid memory types: "decision", "bugfix", "architecture", "preference", "pattern", "session_summary".
4. Use scope "global" ONLY for cross-project user preferences (language, coding style, tooling preference); otherwise use "project".
5. If a new decision replaces an existing memory listed below, set "supersedes_id" to that memory's exact id.
6. Never include secrets, tokens, or credentials.

Respond ONLY with valid JSON matching:
{"memories":[{"scope":"project","type":"decision","title":"<=80 chars","summary":"1-2 concise sentences","content":"Key details, rationale, gotchas","files":["path/to/file"],"tags":["tag"],"supersedes_id":null}]}"#.to_string();

    let mut existing_block = String::new();
    for m in existing_recent.iter().take(6) {
        existing_block.push_str(&format!(
            "- {} [{}]: {} — {}\n",
            m.id, m.memory_type, m.title, m.summary
        ));
    }

    let mut events_block = String::new();
    for ev in events.iter().take(30) {
        let tool = ev.tool_name.as_deref().unwrap_or("-");
        let snippet = truncate_head_tail(&redact_secrets(&ev.payload_json), 900);
        events_block.push_str(&format!(
            "[{} | tool:{}] {}\n",
            ev.event_type, tool, snippet
        ));
        if events_block.len() > 7_500 {
            break;
        }
    }

    let user = format!(
        "Project: {project_name}\nSource AI client: {source_client}\n\nExisting recent memories (avoid duplicates; set supersedes_id if replaced):\n{}\nTurn events:\n{}",
        if existing_block.is_empty() {
            "(none)\n"
        } else {
            &existing_block
        },
        events_block
    );

    (system, user)
}

pub fn parse_extraction_json(raw: &str) -> Result<ExtractionEnvelope> {
    let trimmed = raw.trim();
    if let Ok(env) = serde_json::from_str::<ExtractionEnvelope>(trimmed) {
        return Ok(env);
    }
    // Extract from ```json ... ``` or outermost { ... }
    let candidate = if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                &trimmed[start..=end]
            } else {
                trimmed
            }
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    serde_json::from_str::<ExtractionEnvelope>(candidate).with_context(|| {
        format!(
            "Failed to parse extraction JSON: {}",
            truncate_head_tail(trimmed, 240)
        )
    })
}

pub fn estimate_cost_usd(provider: &str, model: &str, in_tokens: i64, out_tokens: i64) -> f64 {
    let m = model.to_ascii_lowercase();
    if provider == "ollama" || provider.starts_with("cli_") || m.ends_with(":free") {
        return 0.0;
    }
    // Approximate USD per 1M tokens (input, output) for common fast distillation models
    let (in_rate, out_rate) = if m.contains("flash-lite") || m.contains("nano") {
        (0.10, 0.40)
    } else if m.contains("flash") || m.contains("mini") || m.contains("deepseek") {
        (0.25, 1.00)
    } else if m.contains("haiku") {
        (0.80, 4.00)
    } else {
        (0.50, 2.00)
    };
    ((in_tokens as f64 / 1_000_000.0) * in_rate) + ((out_tokens as f64 / 1_000_000.0) * out_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_and_raw_extraction_json() {
        let fenced = "```json\n{\"memories\":[{\"scope\":\"project\",\"type\":\"bugfix\",\"title\":\"Fix CORS header\",\"summary\":\"Added localhost origin\",\"content\":\"Allowed 127.0.0.1\",\"files\":[\"src/server.rs\"],\"tags\":[\"cors\"],\"supersedes_id\":null}]}\n```";
        let env = parse_extraction_json(fenced).unwrap();
        assert_eq!(env.memories.len(), 1);
        assert_eq!(env.memories[0].title, "Fix CORS header");
    }

    #[test]
    fn free_and_ollama_models_estimate_zero_cost() {
        assert_eq!(
            estimate_cost_usd("ollama", "qwen2.5-coder:7b", 5000, 500),
            0.0
        );
        assert_eq!(
            estimate_cost_usd("openrouter", "cohere/north-mini:free", 5000, 500),
            0.0
        );
        assert!(
            estimate_cost_usd("openrouter", "google/gemini-2.5-flash-lite", 10_000, 1_000) > 0.0
        );
    }
}
