# `0x1-ai-memory` — Architecture & Design Specification

> **Goal:** A local-first, open-source, cross-agent persistent memory engine shipped as a **single Rust binary (`0x1-memory`)** with an embedded localhost Web UI. Connects **Claude Code**, **Codex CLI/App**, **Antigravity**, and any MCP client to one unified, ultra-lean, anti-drain memory store.

Primary research backing every integration detail:
- [`docs/research/claude-mem-analysis.md`](research/claude-mem-analysis.md) — source audit of `thedotmack/claude-mem` v16.1.2 (6 quota-drain root causes, schema bloat, 3-runtime friction).
- [`docs/research/client-integrations.md`](research/client-integrations.md) — primary-source verification & T3 Code (`0.1.11`) `app.asar` reverse engineering.

---

## 1. Core Design Principles (Anti-`claude-mem` Invariants)

1. **Single Binary, Zero Extra Runtimes:** One Rust executable (`0x1-memory`) provides the background daemon (`axum`), Streamable HTTP & stdio MCP server, <5ms hook CLI shims, SQLite + FTS5 + `sqlite-vec` storage, lazy in-process ONNX embeddings (`fastembed`), and the embedded React/Vite Web UI (`rust-embed`). No Python, no `uvx`, no Chroma sidecar, no Node/Bun required.
2. **Hybrid Per-Turn Anti-Drain Capture:**
   - During active coding, hook shims record raw events to SQLite in **<5ms at 0 LLM tokens** (deterministic secret scrubbing + head/tail truncation).
   - Background LLM distillation runs **once per completed turn (`Stop`)**, on idle timeout / crash recovery, or on `PreCompact` — using a **stateless compact prompt** (never $O(N^2)$ conversation replay, never per-`Read`/`Grep` firing, never hidden oversized-field compression sub-calls).
   - Agents and users can also call MCP `memory_save` explicitly at any time.
3. **Decoupled Memory Engine & Strict Zero Silent Fallback:**
   - Default providers: Direct API / Local LLM (**OpenRouter**, **Gemini**, **OpenAI-compatible / Ollama**).
   - **Zero-LLM Safe Mode:** Without any API key, raw capture, hybrid search (FTS5 + local ONNX), and explicit `memory_save` work seamlessly.
   - **Opt-In CLI Passthrough:** `claude -p`, `codex exec`, and `agy -p` are strictly opt-in with mandatory recursion guard (`OX1_MEMORY_INTERNAL=1`) and isolation flags (`--no-session-persistence` / `--ephemeral`, hooks & MCP disabled).
   - **Hard Invariant:** Never silently fall back to a user's Claude/Codex/Antigravity coding subscription when an API provider errors or hits rate limits. Distillation, API embeddings, and provider tests check recorded UTC-day usage before dispatch and record the attempt in `usage_ledger`. Each limit can be disabled with `0`. An in-flight call can exceed the threshold.
4. **Ultra-Lean Progressive Disclosure Recall:**
   - `SessionStart` injects a compact **~300–500 token briefing** (pinned memories + 5–8 recent cross-AI headlines for this project + global user preferences).
   - Every Briefing, prompt Recall and MCP `memory_search` result is time-anchored (`src/clock.rs`): a `Generated at <local time> UTC<offset>` line, and per memory its last-updated local time with its age relative to that moment, e.g. `19:06, 14m before this briefing`. Agents can then tell minutes ago from yesterday.
   - Exactly **5 compact MCP tools**: `memory_search`, `memory_get`, `memory_timeline`, `memory_save`, `memory_update`.
   - No intrusive `PreToolUse` File Read Gate.
5. **Cross-Agent by Default & Clean Scoping:**
   - **2-Tier Scope:** `project` (unified by normalized git remote URL or canonical repo path so worktrees & all 4 clients hit the same project) + `global` (personal coding style/preferences across repos).
   - Cross-client sharing is **ON by default**; every memory carries `source_client` (`claude-code` | `codex` | `antigravity` | `mcp`) and lifecycle status (`active` | `pinned` | `superseded` | `archived`).
6. **Security & Privacy at the Boundary:**
   - Deterministic secret redaction (API keys, Bearer tokens, `.env` assignments, JWTs, PEM private keys, `<private>...</private>` blocks) runs **before** persistence or LLM dispatch.
   - Per-project `privacy_mode`: `full` (local DB + configured providers), `local_only` (local DB, on-device embeddings, and eligible loopback providers), or `paused` (no capture). Use trusted loopback servers that perform inference on-device for `local_only`.

---

## 2. System Topology

```text
  Claude Code        Codex CLI / App       Antigravity
  (~/.claude)          (~/.codex)          (~/.gemini)
   │      ▲             │      ▲            │      ▲
   │hooks │MCP          │hooks │MCP         │hooks │MCP
   ▼      │             ▼      │            ▼      │
 ┌─────────────────────────────────────────────────────────────────────────┐
 │              `0x1-memory` Single Rust Binary (127.0.0.1:10101)          │
 │                                                                         │
 │  CLI Hook Shim (<5ms) ──► POST /v1/hooks/{claude|codex|antigravity}     │
 │  Streamable HTTP MCP  ──► POST /mcp (5 tools, JSON-RPC 2.0)             │
 │  REST API & SSE       ──► /api/*  &  /api/stream                        │
 │  Embedded Web UI      ──► /*  (React + Vite + Tailwind via rust-embed)  │
 │                                                                         │
 │  ┌──────────────┐  ┌─────────────────────┐  ┌────────────────────────┐ │
 │  │ Secret Scrub │─►│ SQLite (WAL)        │◄─│ Async Distill Worker   │ │
 │  │ & Truncator  │  │  • 7 core tables    │  │  • Per-turn batching   │ │
 │  └──────────────┘  │  • FTS5 (BM25)      │  │  • Budget Guardrail    │ │
 │                    │  • sqlite-vec (384) │  │  • OpenRouter/Gemini/  │ │
 │                    │  • fastembed (ONNX) │  │    Ollama/Opt-in CLI   │ │
 │                    └─────────────────────┘  └────────────────────────┘ │
 └─────────────────────────────────────────────────────────────────────────┘
```

- **Default data directory:** `~/.0x1-memory/` (`memory.db`, `backups/`, `models/`, `daemon.pid`, `daemon.log`).
- **Default bind:** `127.0.0.1:10101` (configurable via `--port` or `OX1_MEMORY_PORT`). Localhost-only; never exposed on `0.0.0.0`.

---

## 3. SQLite Storage Schema (8 Core Tables + 2 Search Indexes)

Designed to stay clean and cloud-sync-ready (UUIDv7/ULID-friendly string IDs, explicit timestamps, soft lifecycle states):

1. **`projects`**: `id`, `name`, `remote_url`, `root_paths_json`, `privacy_mode` (`full|local_only|paused`), `created_at`, `updated_at`.
2. **`sessions`**: `id`, `project_id`, `source_client`, `external_session_id` (unique per client), `branch`, `started_at`, `ended_at`, `last_turn_at`, `status` (`active|ended`).
3. **`raw_events`**: `id`, `session_id`, `project_id`, `source_client`, `event_type` (`prompt|tool_use|tool_error|turn_stop`), `tool_name`, `payload_json` (redacted & truncated), `distilled` (`0|1`), `created_at`. Processed events expire after `raw_event_retention_days` (default 14). Pending events can stay for twice that duration.
4. **`memories`**: `id`, `scope` (`project|global`), `project_id`, `session_id`, `source_client`, `memory_type` (`decision|bugfix|architecture|preference|pattern|session_summary`), `title`, `summary`, `content`, `files_json`, `tags_json`, `status` (`active|pinned|superseded|archived`), `superseded_by`, `created_at`, `updated_at`.
   - **`memories_fts`**: FTS5 (`unicode61`) virtual table synced via triggers on `title`, `summary`, `content`, `tags_json`, `files_json`.
   - **`memories_vec`**: `sqlite-vec` `vec0` virtual table storing `float[384]` embeddings (`AllMiniLML6V2Q` / `BGESmallENV15Q` via `fastembed`, or API embeddings).
   - **Hybrid Ranking:** Reciprocal Rank Fusion (`RRF(k=60)`) combining BM25 rank + cosine vector distance, boosted for `pinned` status and recency, excluding `superseded`/`archived` by default.
5. **`usage_ledger`**: `id`, `provider`, `model`, `operation` (`distill|embed|test`), `input_tokens`, `output_tokens`, `estimated_cost_usd`, `session_id`, `status` (`ok|error|budget_blocked`), `error_message`, `created_at`.
6. **`client_connections`**: `client_id` (`claude-code|codex|antigravity`; other names are not Clients and get no row), `detected`, `connected`, `needs_manual_step`, `manual_step_hint`, `config_backup_path`, `last_heartbeat_at`, `last_event_at`, `updated_at`.
7. **`settings`**: Key-value table for ~12 essential settings:
   - `provider` (`none|openrouter|gemini|openai_compatible|ollama|cli_claude|cli_codex|cli_agy`)
   - `model` (e.g. `google/gemini-2.5-flash-lite` or `qwen2.5-coder:7b`)
   - `base_url`
   - `api_key` (or an `env:VAR` reference), reused only for the same provider and endpoint
   - `daily_token_cap` (default `250000`, `0` disables the token limit)
   - `daily_cost_cap_usd` (default `0.50`, `0` disables the USD limit)
   - `session_start_token_budget` (default `450`, approximate size of the memory briefing injected at session start)
   - `embedding_mode` (`local_onnx|fts_only|api`)
   - `embedding_model`
   - `raw_event_retention_days` (default `14`)
   - `distill_idle_seconds` (default `120`, pending sessions become eligible for recovery after this idle period without a turn-end signal)
   - `onboarding_completed` (`true|false`, auto-seeded `true` once for upgraded installs with existing data or client configs)
8. **`memory_recall_stats`**: `memory_id`, `recall_count`, `last_recalled_at`, `clients_json` — lightweight cross-agent recall telemetry incremented only when `SessionStart`, `UserPromptSubmit`/`PreInvocation`, `memory_search`, or `memory_get` serves a memory.

---

## 4. Client Integrations & CLI Setup

Every connect operation generates a dry-run diff preview, writes timestamped backups to `~/.0x1-memory/backups/`, and supports clean 1-click disconnect/rollback.

| Client | MCP Wiring | Hook Wiring | Special Handling |
| :--- | :--- | :--- | :--- |
| **Claude Code** | `~/.claude.json` → `mcpServers["0x1-memory"] = { "type": "http", "url": "http://127.0.0.1:10101/mcp" }` | `~/.claude/settings.json` → `SessionStart`, `UserPromptSubmit`, `PostToolUse` (`Edit\|Write\|MultiEdit\|Bash`), `PreCompact`, `Stop`, `SessionEnd` | Auto-inherited inside T3 Code. Outputs `hookSpecificOutput.additionalContext`. |
| **Codex CLI / App** | `~/.codex/config.toml` (`toml_edit`) → `[mcp_servers.0x1_memory]` with `url` & per-tool `approval_mode = "approve"` for read tools | `~/.codex/hooks.json` (`[features] hooks = true`) with static command `<abs-bin> hook codex` | Auto-inherited inside T3 Code. **Never** emits top-level `systemMessage`. Onboarding guides user to run `/hooks` once for SHA-256 trust. |
| **Antigravity** | Atomic plugin `~/.gemini/config/plugins/0x1-memory/` (`plugin.json`, `mcp_config.json`, root `hooks.json`, `rules/AGENTS.md`) | `SessionStart`, `PreInvocation`, `PostToolUse` (`run_command\|write_file\|edit_file`), `Stop` | Outputs camelCase `injectSteps[{systemMessage:{systemMessage, metadata:{user_markdown}}}]`. Deduplicates `PreInvocation` by `(conversationId, lastUserInput)`. |
| **Hosts (T3 Code, Orca)** | Inherited for Claude & Codex | None: the Daemon never reads a Host's storage | Antigravity inside T3 Code, and Orca's Antigravity chat with a non-Google sign-in, run with a private `GEMINI_HOME` and are not captured (`docs/research/client-integrations.md` §4.5). |
| **Universal MCP** | Copy-paste JSON/TOML for Streamable HTTP (`http://127.0.0.1:10101/mcp`) + `0x1-memory mcp-stdio` fallback | N/A | Works with Cursor, Windsurf, OpenCode, Zed, Cline, etc. |

---

## 5. MCP Tool Surface (5 Compact Tools)

Total tool definition footprint kept under ~550 tokens:

1. **`memory_search(query, project?, scope?, type?, client?, limit?)`** — Hybrid FTS5 + vector search returning compact rows (`id`, `title`, `summary`, `type`, `source_client`, `status`, `updated_at`, `score`).
2. **`memory_get(ids[])`** — Fetches full `content`, `files`, and `tags` for specific memory IDs.
3. **`memory_timeline(memory_id, before?, after?)`** — Returns chronological neighboring memories around an anchor ID.
4. **`memory_save(title, summary, content, type, scope?, project?, files?, tags?, supersedes_id?)`** — Explicitly stores a curated memory (and optionally supersedes an older memory).
5. **`memory_update(id, status?, title?, summary?, content?, superseded_by?)`** — Pin, edit, archive, or supersede an existing memory.

---

## 6. Embedded Web UI (1 SPA, Onboarding + 4 Dashboard Pages)

0. **Onboarding (`/onboarding`)**: Distraction-free 4-step first-run flow (Welcome → AI Clients with combined diff preview → Memory Engine → Ready with opt-in Auto-Start toggle). Auto-skipped once for upgraded users; re-runnable anytime.
1. **Live Stream (`/`)**: Split 2-column Activity Console — left column shows real-time SSE cross-agent memories grouped by Day/Session/Flat with clickable tags/files and drawer inspection + **Live Token & Cost Meter**; right collapsible **Pipeline Rail** shows active/recent sessions with 1-click `Distill now` / `Flush pending` and a pauseable, aggregated **Live Event Tail**.
2. **Memories (`/memories`)**: True backend hybrid search (FTS5 + vector), multi-axis grouping (`Project` / `Type` / `Session` / `File` / `Tag` / `Flat`), recall-hit sorting, interactive Canvas 2D + `d3-force` **Graph Map** with zero-LLM derived edges, a unified **Right Inspector Drawer** (edit, supersession lineage, timeline, 1-hop SVG neighborhood), and a **Briefing Preview** modal with token budget breakdown and progressive disclosure savings.
3. **Connections (`/connections`)**: Auto-detected client cards with live heartbeat status, Port Mismatch detection & 1-click Reconnect, dry-run diff preview before connect, 1-click Connect/Disconnect/Test Ping, Codex `/hooks` 1-step callout, T3 watcher toggle, and Universal MCP snippet.
4. **Engine & Budget (`/engine`)**: Active provider and UTC-day usage, provider and model draft settings, a synthetic distillation test, independent limits, search and briefing controls, and storage and recovery settings. The usage ledger records distillation, API embeddings, and tests. Daemon controls cover updates, login Auto-Start, and confirmed shutdown.

---

## 7. CLI Commands

- `0x1-memory` / `0x1-memory start [--port 10101] [--open]` — Start detached background Daemon (`~/.0x1-memory/daemon.pid` + `daemon.log`), idempotent if already running, and optionally open Onboarding/Web UI.
- `0x1-memory stop [--port 10101]` — Stop the background Daemon cleanly (tears down active SSE streams immediately).
- `0x1-memory restart [--port 10101] [--open]` — Restart the background Daemon.
- `0x1-memory logs [-n 50]` — Print recent lines from `~/.0x1-memory/daemon.log`.
- `0x1-memory autostart <enable|disable|status>` — Opt-in login Auto-Start (`launchd` on macOS, `systemd --user` on Linux).
- `0x1-memory serve [--port 10101] [--open] [--foreground]` — Run Daemon in foreground (HTTP API + MCP + SSE + embedded Web UI + background distillation worker).
- `0x1-memory setup [--client all|claude-code|codex|antigravity] [--yes]` — CLI Setup with diff preview & backup.
- `0x1-memory status` — Show Daemon health, Auto-Start, connected/mismatched clients, memory count, and today's token/cost usage.
- `0x1-memory hook <claude|codex|antigravity> [event]` — Ultra-fast (<5ms) stdin→HTTP→stdout hook shim with `OX1_MEMORY_INTERNAL=1` recursion guard.
- `0x1-memory mcp-stdio` — Optional stdio MCP bridge forwarding JSON-RPC to the local Daemon (or direct SQLite fallback).
- `0x1-memory search <query>` — Quick terminal hybrid search.
