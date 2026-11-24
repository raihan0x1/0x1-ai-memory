# Primary-Source Technical Analysis: `thedotmack/claude-mem`

> **Snapshot Verified**: `thedotmack/claude-mem` **`v16.1.2`** (commit `2c57eb156ee726c8f0f22502cc9bfcc0a8353526`, 2026-11-23) and official Mintlify docs bundled in `docs/public/` (published at `https://docs.claude-mem.ai/`).  
> **Purpose**: Inform the design of a lean, cross-agent, low-overhead memory system (`0x1-ai-memory`) by auditing every mechanism, schema, LLM call site, provider path, and UX bottleneck in `claude-mem`.

---

## 1. System Architecture

### 1.1 Dual-Runtime Process Topology

`claude-mem` operates in one of two runtimes selected via `CLAUDE_MEM_RUNTIME` (`src/shared/SettingsDefaultsManager.ts:854`):

1. **Worker Runtime (`worker`, default)** — Local single-user daemon (`src/services/worker-service.ts:331`, bundled to `plugin/scripts/worker-service.cjs`):
   - **HTTP + SSE Daemon**: Express 5 (`express@^5.2.1`) running on **Bun** (`bun:sqlite` native driver). Exposes ~125+ REST/SSE endpoints across 20 route handlers registered in `WorkerService.registerRoutes()` and `initializeBackground()` (`src/services/worker-service.ts:904-979, 1238-1390`).
   - **Dynamic Per-User Ports** (`src/shared/install-port.ts:7-86`): Contrary to older documentation citing a fixed port `37777`, `v16.1.2` computes deterministic per-user ports:
     - Worker port: `37700 + (uid % 100)` on POSIX (e.g., UID 501 on macOS → `37701`; UID 1000 → `37700`), or `37700 + (sha256(dataDir) % 100)` on Windows (preserving `37777` only for pre-existing Windows installs). Bound by default to `CLAUDE_MEM_WORKER_HOST = '127.0.0.1'` (`SettingsDefaultsManager.ts:555-556`).
     - Server port: `37877 + offset` (legacy Windows `37954`).
   - **Lifecycle & Process Supervision**: Auto-started asynchronously on `SessionStart` via `claude-mem start` (`plugin/hooks/hooks.json:20-33`). Managed by an internal process supervisor (`src/supervisor/index.ts`, `process-registry.ts`) tracking spawned child processes (`claude` CLI observers, `codex` app-servers, `uvx chroma-mcp`, deep-search child). Opt-in idle shutdown (`CLAUDE_MEM_IDLE_EXIT_SEC`, default `'0'` = stays resident indefinitely; minimum `60s` when enabled, `worker-service.ts:507-573`, `SettingsDefaultsManager.ts:576`).
   - **Background Daemons Inside the Worker**:
     - `ChromaMcpManager` (`src/services/sync/ChromaMcpManager.ts:126-136`): Spawns `nice -n 10 uvx chroma-mcp==0.2.6` over MCP stdio (`StdioClientTransport`) for vector search.
     - `AutoRefreshWorker` (`src/services/work-view/auto-refresh-worker.mjs:44-107`): Polls SQLite every 10 minutes (`CLAUDE_MEM_WORK_VIEW_REFRESH_INTERVAL_MIN='10'`) and launches background LLM agent runs for "My Work" when work is due (`CLAUDE_MEM_WORK_VIEW_AUTO_REFRESH='true'`).
     - Optional `CloudSync` (`src/services/sync/CloudSync.ts`), `TranscriptWatcher` (`src/services/transcripts/watcher.ts`), `BackupManager` (`src/services/backup/BackupManager.ts`), and `HookSpoolDrain` (`src/services/worker/hook-spool-drain.ts`).
2. **Server Runtime (`server`)** — Hosted / multi-tenant mode (`src/server/runtime/ServerService.ts`, `src/server/runtime/ActiveServerGenerationWorkerManager.ts`):
   - Express HTTP server + Postgres (`pg`) with `pg_textsearch` BM25 + `pgvector` HNSW + Redis/BullMQ (`bullmq`, `ioredis`) job queue (`ProviderObservationGenerator`) + `better-auth` API keys (`src/storage/postgres/schema.ts`).
   - Uses in-process ONNX embeddings (`LocalEmbedder` via `@huggingface/transformers`, `src/services/vector/LocalEmbedder.ts:40-60`) instead of `chroma-mcp`.

```
Coding Client (Claude Code / Cursor / Codex / Antigravity / OpenCode / ...)
  ├── Lifecycle Hooks ──▶ claude-mem CLI launcher (plugin/scripts/claude-mem-launcher.cjs)
  │                         ├── HTTP POST/GET ──▶ Worker Daemon (Bun + Express on 127.0.0.1:377xx)
  │                         └── Disk Spool fallback when worker down (src/cli/spool-hook-event.ts)
  ├── MCP Stdio Server ──▶ node plugin/scripts/mcp-server.cjs (27 tools, filtered by runtime/flags)
  │                         ├── Proxies memory/search/work-state/corpus tools ──▶ Worker HTTP API
  │                         └── Executes smart_outline/smart_unfold/smart_search locally (tree-sitter)
  └── Worker Daemon (src/services/worker-service.ts)
        ├── SQLite (bun:sqlite, WAL, 38 tables, FTS5) ──▶ ~/.claude-mem/claude-mem.db
        ├── Chroma Sidecar (uvx chroma-mcp 0.2.6 via stdio MCP) ──▶ ~/.claude-mem/chroma/
        ├── Observer LLM Providers (Claude SDK child / Codex app-server / OpenRouter / Gemini / OpenAI-compat)
        ├── My Work AutoRefreshWorker (periodic background agent runs)
        └── Web UIs: "/" (My Work Vite/React) + "/viewer" (Memory Viewer esbuild/React + SSE /stream)
```

*Docs reference*: `docs/public/architecture/overview.mdx` (`https://docs.claude-mem.ai/architecture/overview`), `docs/public/architecture/worker-service.mdx` (`https://docs.claude-mem.ai/architecture/worker-service`).

### 1.2 Registered Lifecycle Hooks (Claude Code Primary Manifest)

Declared in `plugin/hooks/hooks.json:1-184` (plus an in-process Claude Code JS hooks module `plugin/hooks/hooks-module.js` built from `src/mods/hooks-module.ts:1-25`):

| Hook Event | Matcher | Command / Handler | Sync/Async | Timeout | Purpose |
|---|---|---|---|---|---|
| `Setup` | `*` | POSIX shell wrapper → `node .../scripts/version-check.js` | Sync | 300s | Dependency check & install marker (`hooks.json:7-18`) |
| `SessionStart` (1) | `startup\|resume\|clear\|compact` | `claude-mem start` | Async | 60s | Auto-start worker daemon (`hooks.json:21-33`) |
| `SessionStart` (2) | `startup\|clear\|compact` | `claude-mem hook claude-code context` | **Sync** | 60s | Calls `GET /api/context/inject` → returns `hookSpecificOutput.additionalContext` (`hooks.json:35-48`, `src/cli/handlers/context.ts:459-731`) |
| `SessionStart` (3) | `startup` | `node ${CLAUDE_PLUGIN_ROOT}/scripts/ensure-launcher.cjs` | **Sync** | 180s | Self-heals/places on-PATH `claude-mem` launcher (`hooks.json:50-62`) |
| `UserPromptSubmit` | *(all)* | `claude-mem hook claude-code session-init` | **Sync** | 15s | Calls `POST /api/sessions/init`; optionally calls `/api/context/semantic` (`hooks.json:64-78`, `src/cli/handlers/session-init.ts:87-383`) |
| `PreToolUse` | `Read\|^read_file$\|^Bash$\|^Grep$\|^Glob$\|^memory_search$\|^memory$\|^mcp__.+__memory_search$` | `claude-mem hook claude-code file-context` | **Sync** | 15s | Injects per-file observation history via `additionalContext`; **denies full-file `Read` ≥ 32 KiB** when `CLAUDE_MEM_FILE_READ_GATE_ENABLED=true` (`hooks.json:116-131`, `src/cli/handlers/file-context.ts:50,333-485`) |
| `PostToolUse` | `*` | `claude-mem hook claude-code observation` | Async | 120s | Posts tool name, input, response, `cwd` to `POST /api/sessions/observations` (`hooks.json:80-96`, `src/cli/handlers/observation.ts:52-150`) |
| `PostToolUseFailure` | `*` | `claude-mem hook claude-code observation` | Async | 120s | Same observation capture path for failed tool executions (`hooks.json:98-115`) |
| `Stop` | *(all)* | `claude-mem hook claude-code summarize` | Async | 120s | Reads transcript tail, posts `last_assistant_message` to `POST /api/sessions/summarize` (`hooks.json:133-149`, `src/cli/handlers/summarize.ts:107-272`) |
| `SubagentStop` | *(all)* | `claude-mem hook claude-code subagent-stop` | Async | 30s | Records subagent completion (`hooks.json:150-165`) |
| `SessionEnd` | *(all)* | `claude-mem hook claude-code session-end` | Async | default | Marks session completed via `POST /api/sessions/session-end` (`hooks.json:167-182`) |
| JS Hooks Module | `session.start` / mods | `plugin/hooks/hooks-module.js` (`src/mods/hooks-module.ts`) | In-proc | N/A | Registers `auto-clear` mod (off by default; triggers `/clear` at `40%` context) and `session-start-card` TUI mod (`src/mods/hooks-module.ts:21-25`) |

*Docs reference*: `docs/public/architecture/hooks.mdx` (`https://docs.claude-mem.ai/architecture/hooks`).

### 1.3 MCP Server Tools & Progressive Disclosure Workflow

`plugin/.mcp.json:1-12` registers a single stdio MCP server (`mcp-search`) launched via an inline Node bootstrap script that locates and `require()`s `plugin/scripts/mcp-server.cjs` (source: `src/servers/mcp-server.ts`, 1,458 lines).

**27 tools** are defined in `src/servers/mcp-server.ts:528-1185` and dynamically filtered by runtime and feature flags in `src/servers/mcp-tool-visibility.ts:4-75`:

| Group | Tools (Line in `src/servers/mcp-server.ts`) | Visibility / Notes |
|---|---|---|
| **3-Layer Progressive Recall** | `important_workflow` (L593), `search` (L637), `timeline` (L679), `get_observations` (L709), `get_tool_uses` (L737) | Core workflow; `get_tool_uses` is worker-only (`mcp-tool-visibility.ts:21`). |
| **Fast / Explicit Memory** | `dumb_search` (L528), `save_memory` (L566), `session_start_context` (L834) | `dumb_search` & `save_memory` are worker-only (`mcp-tool-visibility.ts:21`). |
| **Working State** | `work_state_write` (L762), `work_state_read` (L783) | Advertised by default; hidden only if `CLAUDE_MEM_WORK_STATE_ENABLED=false` (`mcp-tool-visibility.ts:39-56`). |
| **AST Code Reading (Local)** | `smart_search` (L944), `smart_unfold` (L992), `smart_outline` (L1043) | Runs tree-sitter directly in the MCP process against workspace files (paired with the File Read Gate, `mcp-server.ts:9,512`). |
| **Knowledge Corpora** | `build_corpus` (L1080), `list_corpora` (L1104), `prime_corpus` (L1117), `query_corpus` (L1134), `rebuild_corpus` (L1152), `reprime_corpus` (L1170) | Proxied over SSE heartbeats to `/api/corpus/*` (`mcp-server.ts:129-166`). |
| **TeamBrain** | `team_brief` (L800), `team_search` (L816) | Hidden until `CLAUDE_MEM_TEAMBRAIN_URL` is configured (`mcp-tool-visibility.ts:58-75`). |
| **Server-Mode Only** | `observation_add` (L858), `observation_record_event` (L875), `observation_search` (L897), `observation_context` (L914), `observation_generation_status` (L930) | Hidden in `worker` runtime (`mcp-tool-visibility.ts:4-13,36`). |

*(Separately, hosted server mode exposes a compact 5-tool recall server in `src/server/mcp/recall-mcp-server.ts:44-110`: `search`, `timeline`, `get_observations`, `context`, `recent`.)*

**Progressive Disclosure 3-Layer Workflow** (`src/servers/mcp-server.ts:593-736`, `docs/public/progressive-disclosure.mdx` → `https://docs.claude-mem.ai/progressive-disclosure`):
1. **Layer 1 — `search`**: Calls `GET /api/search` (`SearchRoutes.ts:236`) via `SearchOrchestrator` (Chroma semantic search or SQLite FTS5) and returns a compact index table containing only IDs, titles, types, dates, and token-size hints (~50–100 tokens/row).
2. **Layer 2 — `timeline`**: Calls `GET /api/timeline` (`SearchRoutes.ts:238`, `SearchManager.ts:865`) given an `anchor` observation ID (or query) plus `depth_before` / `depth_after` to inspect neighboring events chronologically across observations, summaries, and prompts.
3. **Layer 3 — `get_observations`**: Calls `POST /api/observations/batch` (`DataRoutes.ts:243`) to fetch full narratives, facts, concepts, and file paths only for selected IDs. An optional Layer 4 (`get_tool_uses`, L737) fetches raw `tool_input`/`tool_response` from `tool_uses`.
- **`dumb_search`** (`src/services/worker/ProgressiveMemorySearch.ts:27-128`): Single-call non-reasoning shortcut running the same 3 steps automatically (candidate search window `max(limit, 100)`, title word-overlap reranking, timeline fetch, and up to 5 observation + 5 summary details).

### 1.4 Web UI Stack & Architecture

The worker serves **two completely separate React 19 frontends** (`src/services/worker/http/routes/ViewerRoutes.ts:48-49, 225-237`):
1. **Primary Root UI (`/` — "My Work")**: Built with **Vite + React 19 + Tailwind CSS + shadcn/Radix UI + Zustand + Immer + Motion + tsParticles + `tokenlens` + Vercel `ai` SDK** (`src/ui/my-work/package.json`, **246 files** in `src/ui/my-work/src/`), compiled to `plugin/ui/work-view/index.html` and backed by **47 HTTP endpoints** in `src/services/worker/http/routes/WorkViewRoutes.ts` (2,080 lines). Provides goal grouping, request progress stories, focus areas, deliverables, and interactive browser setup (`/?setup=1`).
2. **Memory Viewer (`/viewer`)**: Separate React 19 SPA (`src/ui/viewer/App.tsx`, 709 lines) bundled by **esbuild** (`scripts/build-viewer.js:17-33`) into an IIFE (`plugin/ui/viewer-bundle.js` + `plugin/ui/viewer.html`). Connects to `GET /stream` via `EventSource` SSE (`src/ui/viewer/hooks/useSSE.ts`) for live observation/summary/prompt pushes, infinite-scroll pagination (`usePagination.ts`), Logs drawer (`LogsModal.tsx`, 767 lines), and Settings modal (`ContextSettingsModal.tsx`, 1,099 lines).

---

## 2. Exact Database & Data Schemas

### 2.1 Worker SQLite Database (`~/.claude-mem/claude-mem.db`)

Opened via `bun:sqlite` in `src/services/sqlite/connection.ts:1,124-143` with pragmas:
- `PRAGMA busy_timeout = 5000` (configured constant `SQLITE_BUSY_TIMEOUT_MS`)
- `PRAGMA foreign_keys = ON`
- `PRAGMA synchronous = NORMAL`
- `PRAGMA journal_size_limit = ...`
- `PRAGMA auto_vacuum = INCREMENTAL`
- `PRAGMA journal_mode = WAL`

**Migration System**: Hand-rolled imperative idempotent migrations inside `SessionStore` (`src/services/sqlite/SessionStore.ts`, 5,653 lines) tracked via `schema_versions(id, version UNIQUE, applied_at)` and guarded by `PRAGMA user_version` (`LATEST_SCHEMA_VERSION = 75`, `src/services/sqlite/connection.ts:19,75`).  
Instantiating `new SessionStore(':memory:')` + `new SessionSearch(store.db)` directly under Bun confirms **`v16.1.2` creates 38 tables, 86 indexes, and 12 triggers** (60 recorded version entries up to version `75`).

#### Core Memory & Queue Tables (Verified via Live `PRAGMA table_info`)

| Table | Cols | Exact Columns & Primary Key / Defaults | Defined / Altered In |
|---|---|---|---|
| `sdk_sessions` | 18 | `id` [PK], `content_session_id`, `memory_session_id` (UNIQUE), `project`, `platform_source='claude'`, `user_prompt`, `started_at`, `started_at_epoch`, `completed_at`, `completed_at_epoch`, `status='active'` (`active\|completed\|failed`), `worker_port`, `prompt_counter=0`, `custom_title`, `observed_model`, `observed_billing`, `cwd`, `project_key_source` | `SessionStore.ts:1310-1322`, altered at L577, 1376, 1388, 2049, 2068, 2113-2116, 2583, 2602 |
| `observations` | 31 | `id` [PK], `memory_session_id` (FK→`sdk_sessions`), `project`, `text`, `type`, `title`, `subtitle`, `facts` (JSON array), `narrative`, `concepts` (JSON array), `files_read` (JSON array), `files_modified` (JSON array), `prompt_number`, `created_at`, `created_at_epoch`, `discovery_tokens=0`, `content_hash`, `generated_by_model`, `relevance_count=0`, `merged_into_project`, `agent_type`, `agent_id`, `metadata` (JSON, e.g., line anchors), `synced_at`, `origin_device_id`, `origin_local_id`, `sync_rev='1'`, `occurrence_count=1`, `title_norm_key`, `reinforcement_dates`, `last_reinforced` | `SessionStore.ts:1330-1339`, altered at L488, 494, 812, 847, 1396, 1608-1614, 1767, 2033, 2093-2096, 2457-2460, 2523, 2763-2766, 2906 |
| `session_summaries` | 20 | `id` [PK], `memory_session_id` (UNIQUE FK→`sdk_sessions`), `project`, `request`, `investigated`, `learned`, `completed`, `next_steps`, `files_read`, `files_edited`, `notes`, `prompt_number`, `created_at`, `created_at_epoch`, `discovery_tokens=0`, `merged_into_project`, `synced_at`, `origin_device_id`, `origin_local_id`, `sync_rev='1'` | `SessionStore.ts:1346-1361`, altered at L812, 847, 1404, 1775, 2533 |
| `user_prompts` | 13 | `id` [PK], `session_db_id` (FK→`sdk_sessions`), `content_session_id`, `prompt_number`, `prompt_text`, `created_at`, `created_at_epoch`, `synced_at`, `origin_device_id`, `origin_local_id`, `sync_rev='1'`, `native_prompt_id`, `native_prompt_hash` | `SessionStore.ts:1734-1743`, altered at L404, 812, 847 |
| `pending_messages` | 16 | `id` [PK], `session_db_id` (FK→`sdk_sessions`), `content_session_id`, `message_type` (`observation\|summarize`), `tool_name`, `tool_input`, `tool_response`, `cwd`, `last_user_message`, `last_assistant_message`, `prompt_number`, `status='pending'` (`pending\|processing`), `created_at_epoch`, `agent_type`, `agent_id`, `tool_use_id` | `SessionStore.ts:1795-1810`, altered at L2776-2801 *(unique index `ux_pending_session_tool` at L2840)* |
| `tool_uses` | 20 | `id` [PK], `tool_use_id`, `content_session_id`, `memory_session_id`, `session_db_id`, `project`, `platform_source='claude'`, `tool_name`, `tool_input`, `tool_response`, `cwd`, `prompt_number`, `agent_type`, `agent_id`, `observation_id`, `or_generation_id`, `or_session_id`, `content_hash`, `created_at`, `created_at_epoch` (`UNIQUE(platform_source, content_session_id, tool_use_id)`) | `src/services/sqlite/tool-uses.ts:160-183` |
| `observation_files` | 2 | `path` [PK], `observation_id` [PK] (FK→`observations` CASCADE) | `SessionStore.ts:369-373` |
| `work_state_entries` | 10 | `id` [PK], `project`, `list_name`, `fields` (JSON), `created_at`, `created_at_epoch`, `synced_at`, `origin_device_id`, `origin_local_id`, `sync_rev='1'` | `src/services/sqlite/work-state.ts:42-50` |
| `deliverables` | 18 | `id` [PK], `project`, `content_session_id`, `platform_source`, `agent_id`, `title`, `kind`, `source_path`, `bucket_path`, `bytes`, `sha256`, `declared=0`, `list_name`, `field`, `tool_use_id`, `occurred_at_epoch`, `created_at`, `created_at_epoch` | `src/services/sqlite/deliverables.ts:58-78` |

#### Supporting Tables (29 Additional Tables)
- **FTS5 Virtual Tables (2 active at init + 1 lazy)**:
  - `observations_fts` (`title, subtitle, narrative, text, facts, concepts`, `content='observations'`, `tokenize='unicode61'`, `SessionSearch.ts:286-296`) with `observations_ai/ad/au` triggers (`SessionSearch.ts:50-65`).
  - `session_summaries_fts` (`request, investigated, learned, completed, next_steps, notes`, `content='session_summaries'`, `tokenize='unicode61'`, `SessionSearch.ts:310-320`) with `session_summaries_ai/ad/au` triggers (`SessionSearch.ts:70-85`).
  - `tool_uses_fts` (`tool_input, tool_response`, `content='tool_uses'`, `tokenize='unicode61'`, lazily created in `src/services/sqlite/tool-use-search.ts:82`).
  - *Note*: `user_prompts_fts` was explicitly **removed** (`dropWriteOnlyUserPromptsFtsAndScopeFtsUpdateTriggers`, `SessionStore.ts:1730-1731`); user prompts are queried via substring matching.
- **Deduplication (3)**: `token_df`, `dedup_meta`, `observation_dedup_candidates` (`SessionStore.ts:499-533`).
- **Media Pipeline (7)**: `media_events`, `media_attachments`, `media_event_refs`, `observation_media_links`, `media_cleanup_jobs`, `media_event_results`, `media_cloud_deletions` (`src/services/sqlite/media.ts:74-124`) + 2 cleanup triggers.
- **Cloud Sync & Recovery (10)**: `sync_state`, `sync_outbox`, `sync_entity_heads`, `sync_content_outbox`, `sync_launch_exclusions`, `sync_dead_letter`, `sync_pull_quarantine`, `sync_recovery_jobs`, `sync_recovery_items`, `sync_recovery_receipts` (`SessionStore.ts:85-146, 873-1085`).
- **Deletion Cutoff & Idempotency (4)**: `hook_spool_consumed`, `committed_observation_tool_uses`, `deleted_content_sessions`, `deleted_memory_sessions` (`SessionStore.ts:2199-2233`) + 4 cutoff triggers preventing deleted sessions/tool-uses from being re-inserted by delayed hooks/sync.
- **Misc (3)**: `schema_versions`, `advisor_calls`, `telegram_wrapups` (`SessionStore.ts:1302, 2425, 2641`).

### 2.2 Vector Store Architecture (Chroma Sidecar vs. Postgres `pgvector`)

`claude-mem` does **NOT** use `sqlite-vec`. It maintains **two completely different vector architectures**:
1. **Worker Runtime — ChromaDB via `uvx chroma-mcp`**:
   - Enabled by default (`CLAUDE_MEM_CHROMA_ENABLED='true'`, `CLAUDE_MEM_CHROMA_MODE='local'`, `SettingsDefaultsManager.ts:705-706`).
   - Spawns an external Python process `uvx chroma-mcp==0.2.6` (using `chromadb 1.5.9` + ONNX `all-MiniLM-L6-v2`, 384-dim, English-only) over MCP stdio (`src/services/sync/ChromaMcpManager.ts:3,47,131`, `src/services/sync/ChromaSync.ts:212,711`). Optional remote mode connects to `127.0.0.1:8000` (`SettingsDefaultsManager.ts:707-709`).
   - `CLAUDE_MEM_CHROMA_EMBEDDING_FUNCTION` defaults to `'default'` (local `all-MiniLM-L6-v2`) and optionally accepts `'openai'`, `'cohere'`, `'jina'`, `'voyageai'`, or `'roboflow'` (`SettingsDefaultsManager.ts:719-725`).
   - `SearchOrchestrator` (`src/services/worker/search/SearchOrchestrator.ts:69-147`) runs semantic vector search via `ChromaSearchStrategy` (recency window default 90 days, `CLAUDE_MEM_SEARCH_RECENCY_DAYS='90'`, batch size `CHROMA_BATCH_SIZE = 100` at `search/types.ts:9`), falling back to `SQLiteSearchStrategy` (FTS5 + substring) when Chroma is disabled or unavailable; file-scoped searches use `HybridSearchStrategy` (SQLite metadata filter → Chroma semantic reranking, `HybridSearchStrategy.ts:15-98`).
2. **Server Runtime — Postgres `pgvector` + `LocalEmbedder`**:
   - Schema (`src/storage/postgres/schema.ts:131-133, 178, 252-536`): `teams`, `projects`, `team_members`, `api_keys`, `audit_log`, `server_sessions`, `agent_events`, `observation_generation_jobs`, `observation_generation_job_events`, `observations` (with `content_search TSVECTOR` GIN index and optional `pg_textsearch` BM25 index), `observation_sources`, `usage_events`, `rate_limit_counters`, and `observation_embeddings` (`embedding vector(dims)` with HNSW index).
   - Generates embeddings in-process via `LocalEmbedder` (`src/services/vector/LocalEmbedder.ts:15-60`) using `@huggingface/transformers` (chunk size 4 to avoid >110ms event-loop stalls, max 32 KB text). Model table (`src/services/vector/models.ts:37-65`, configured by `CLAUDE_MEM_EMBEDDING_MODEL`): defaults to `Xenova/all-MiniLM-L6-v2` (384-dim, 256 tokens) plus multilingual options `paraphrase-multilingual-MiniLM-L12-v2` (384-dim) and `multilingual-e5-small`. *(Note: an intermediate server-SQLite schema also exists in `src/storage/sqlite/schema.ts`, `SERVER_STORAGE_SCHEMA_VERSION = 34`, with `memory_items` + `memory_items_fts`.)*

### 2.3 Structured XML Extraction Schemas

Observation and summary extraction uses **prompt-instructed XML** parsed by regex/stateful tag extraction in `src/sdk/parser.ts:15-416` (NOT JSON Schema / tool-calling structured output).

**Observation XML** (`src/sdk/prompts.ts:43-81`, `plugin/modes/code.json`):
```xml
<observation>
  <type>bugfix | feature | refactor | change | discovery | decision | security_alert | security_note | sensitive</type>
  <title>Short title capturing the core action or topic</title>
  <subtitle>One sentence explanation (max 24 words)</subtitle>
  <facts>
    <fact>Concise, self-contained statement</fact>
  </facts>
  <narrative>Full context: What was done, how it works, why it matters</narrative>
  <concepts>
    <concept>how-it-works | why-it-exists | what-changed | problem-solution | gotcha | pattern | trade-off</concept>
  </concepts>
  <files_read><file>path/to/file</file></files_read>
  <files_modified><file>path/to/file</file></files_modified>
</observation>
```
**No-op / Noise Sentinel** (`src/sdk/parser.ts:106`, `plugin/modes/code.json` `skip_guidance`):
```xml
<skip_summary reason="noise" />
```
**Session Summary XML** (`src/sdk/prompts.ts:498-505`):
```xml
<summary>
  <request>Short title capturing the user's request AND substance</request>
  <investigated>What was explored/examined</investigated>
  <learned>Key discoveries about how things work</learned>
  <completed>What work shipped or changed</completed>
  <next_steps>Current trajectory of active work</next_steps>
  <notes>Additional insights</notes>
</summary>
```
Because extraction relies on free-form XML, `src/sdk/parser.ts:58,352,397-408` carries extensive repair heuristics for drifted tags (e.g., `<kind>`/`<detail>` instead of `<type>`/`<title>`, stray closing tags like `</parameter>` or `</reasoning_effort>`, or leaked placeholder brackets).

---

## 3. Why Quota & Usage Drain Fast (Root Cause Analysis)

Tracing every LLM call site and hook injection across the codebase reveals **six compounding architectural root causes** — four on the **observer (background) side** and two on the **primary coding session side**.

### 3.1 Inventory of Every LLM Call Site in `claude-mem`

| Trigger / Subsystem | Entrypoint (`file:line`) | Provider / Mechanism | Frequency |
|---|---|---|---|
| **1. Every `PostToolUse` & `PostToolUseFailure`** | `plugin/hooks/hooks.json:80-115` → `http/shared.ts:446-475` → `GeneratorRunner.ts:222` | Active observer provider (`ClaudeProvider.ts:537`, `OpenAICompatibleProvider.ts:628`, `OpenRouterProvider.ts:987`, `GeminiProvider.ts:482`, `CodexProvider.ts:401`) | **1 LLM turn per non-skipped tool call** by default (except Codex) |
| **2. Every `Stop` (Assistant Turn End)** | `plugin/hooks/hooks.json:133-149` → `http/shared.ts:578-583` | Same observer provider (`buildSummaryPrompt`, `ClaudeProvider.ts:1420`, `OpenAICompatibleProvider.ts:685-709`) | **1 LLM turn per assistant turn** |
| **3. Oversized Tool Field Condensation** | `src/services/worker/field-optimizer.ts:151-250` → `ClaudeProvider.ts:1207-1233` (`runStandaloneObserverPrompt`, `query()` at L1152) / HTTP `compressField` | **Extra standalone LLM call** (on Claude: spawns a separate `claude` CLI subprocess!) | Fires **before** observation turn whenever `tool_input` or `tool_response` > `16,000` chars (`OBS_PROMPT_FIELD_MAX_CHARS`, `prompts.ts:185`) |
| **4. Output Drift / Empty Reply Re-ask** | `ResponseProcessor.ts:918-960`, `paid-send-budget.ts:11` | Resends claimed batch (`DEFAULT_MAX_PAID_SENDS_PER_BATCH = 2`), plus reasoning-truncation retry (`OpenRouterProvider.ts:1292-1325`) | Up to **2× billed sends** per batch on malformed XML or length-truncated reasoning |
| **5. "My Work" Auto-Refresh (ON by Default!)** | `src/services/work-view/auto-refresh-worker.mjs:1-107`, `src/services/agent-runner/run.ts:214` | Full multi-step agent runs (`owner-goals`, `request-progress`) via Claude SDK (`haiku`/`sonnet`), Codex, Gemini, OpenCode, or API agent (`SettingsDefaultsManager.ts:787-797`) | Checks every **10 minutes**; runs autonomously whenever ungrouped sessions/stale goals exist |
| **6. Optional / On-Demand LLM Calls** | `KnowledgeAgent.ts:262` / `ApiCorpusAgent.ts:245` (`prime_corpus`/`query_corpus`); `ClaudeProvider.ts:1236` (`formatTelegramWrapup`); `UserPromptSubmit` bare-prompt turn if `CLAUDE_MEM_OBSERVE_BARE_PROMPTS=true` (`SettingsDefaultsManager.ts:686`) | Claude SDK `query()` or OpenRouter/OpenAI-compat HTTP | On user/agent invocation (or per prompt if opt-in flag enabled) |

### 3.2 Root Cause 1: 1:1 Per-Tool-Call LLM Fire Hose with Near-Zero Filtering or Batching

- `PostToolUse` and `PostToolUseFailure` both match `*` (`plugin/hooks/hooks.json:82,100`).
- `CLAUDE_MEM_SKIP_TOOLS` defaults to only 5 tools: `'ListMcpResourcesTool,SlashCommand,Skill,TodoWrite,AskUserQuestion'` (`SettingsDefaultsManager.ts:577`). **Every single `Read`, `Grep`, `Glob`, `Bash`, `Edit`, `Write`, `WebFetch`, and MCP tool call** is queued into `pending_messages` and dispatched to the observer LLM.
- `CLAUDE_MEM_SKIP_BASH_PATTERNS` is empty (`''`) and `CLAUDE_MEM_SKIP_SUBAGENT_OBSERVATIONS` defaults to `'false'` (`SettingsDefaultsManager.ts:578-579`), so subagent fan-outs multiply tool events proportionally.
- **No batching by default on primary providers**:
  - `CLAUDE_MEM_CLAUDE_OBSERVATION_BATCH_SIZE: '1'` (`SettingsDefaultsManager.ts:605`, `ClaudeProvider.ts:1468`) — **1 tool call = 1 observer turn**.
  - `CLAUDE_MEM_OPENAI_COMPAT_OBSERVATION_BATCH_SIZE: '1'` (`SettingsDefaultsManager.ts:634`, `OpenAICompatProvider.ts:538`).
  - `OpenRouterProvider` and `GeminiProvider` inherit `OpenAICompatibleProvider.observationTurnPrompt()` (`OpenAICompatibleProvider.ts:539-541`) which **never batches** — strictly 1 HTTP request per `PostToolUse`.
  - Only `CodexProvider` batches by default (`CLAUDE_MEM_CODEX_OBSERVATION_BATCH_SIZE: '8'`, `32,000` chars, `SettingsDefaultsManager.ts:603-604`, `CodexProvider.ts:324-326`).
- On top of every tool call, `Stop` enqueues a `summarize` prompt (`http/shared.ts:578-583`) carrying the full `last_assistant_message` (`src/sdk/prompts.ts:477-511`). A single coding turn with 12 tool calls triggers **13+ observer LLM calls** (plus any field-condensation calls).

### 3.3 Root Cause 2: Quadratic $O(N^2)$ Multi-Turn Context Growth + Cache Thrashing

- **Persistent multi-turn history per observer generation**:
  - `ClaudeProvider` spawns a streaming Claude Code CLI subprocess via `@anthropic-ai/claude-agent-sdk` `query({ prompt: messageGenerator, options })` (`ClaudeProvider.ts:537`). It yields each observation/summary turn into the live CLI process while mirroring every turn in `session.conversationHistory` (`ClaudeProvider.ts:1402,1432`).
  - HTTP providers (`OpenRouterProvider`, `GeminiProvider`, `OpenAICompatProvider`) are stateless HTTP calls that **re-send the entire accumulated `session.conversationHistory` array on every single tool call** (`OpenAICompatibleProvider.ts:618-631`).
- **Payload size per turn**:
  - Framing prompt (`observerInstructions(mode)` + `loadSessionStartContext()`): ~5.5 KB instructions (`plugin/modes/code.json`) + up to **10,000 chars** of prior project memory (`recycle-conversation.ts:45-67`, `ContextBudget.ts:9`).
  - Each tool call appends `<parameters>` (up to `16,000` chars) + `<outcome>` (up to `16,000` chars) = **up to 32 KB (~8,000 tokens) of raw JSON per tool call** (`src/sdk/prompts.ts:173-185, 451-464`) plus ~300–800 tokens of assistant XML output.
  - Because `conversationHistory` grows monotonically until `CLAUDE_MEM_OBSERVER_MAX_CONVERSATION_CHARS = 100_000` chars (~25–26K tokens/turn; previously **`400_000` chars ≈ ~100K tokens/turn** before issue `#5023`, documented directly in `SettingsDefaultsManager.ts:679-681` and `src/shared/observer-recycle.ts:25-27`), a 15-tool-call generation re-reads $\sum_{i=1}^{15} (\text{base} + i \cdot \Delta)$ — routinely **150K–350K input tokens per generation**!
- **Subprocess teardown & cold-cache penalties**:
  - `ClaudeProvider` hardcodes `const shouldResume = false;` (`ClaudeProvider.ts:439`).
  - `SessionMessageBuffer` has `IDLE_TIMEOUT_MS = 3 * 60 * 1000` (3 minutes, `SessionMessageBuffer.ts:6`). Whenever the user pauses >3 minutes to read/think, the Claude observer subprocess exits. On the next tool call, a **brand-new `claude` CLI subprocess** spawns, re-loading Claude Code's built-in system prompt (since `buildHardenedSdkOptions` at `src/sdk/hardened-options.ts:216-244` omits `systemPrompt`, keeping Claude Code's default system prompt alongside `tools: []`) + `observerInstructions` + `loadSessionStartContext()` as a **cold cache write** (`cache_creation_input_tokens`).
  - Every generation recycle (`recycle-conversation.ts:45-96`) likewise discards the process and pays a fresh cold-cache write.

### 3.4 Root Cause 3: Hidden Secondary LLM Calls & Background "My Work" Agent

1. **Oversized Field Condensation (`field-optimizer.ts:151-250`)**: Whenever a `Read` or `Bash` output exceeds `16,000` chars (`OBS_PROMPT_FIELD_MAX_CHARS`), `claude-mem` does **not** simply truncate it — it first invokes an **entire extra LLM pass** (`buildFieldCompressionPrompt`, up to 180s timeout) to summarize the raw field. On `ClaudeProvider`, `compressField()` (`ClaudeProvider.ts:1207-1233`) calls `runStandaloneObserverPrompt()` (`L1127-1183`), spawning a **second concurrent `claude` CLI subprocess** for a single large file read!
2. **My Work Auto-Refresh (`auto-refresh-worker.mjs:1-40`)**: Enabled **by default** (`CLAUDE_MEM_WORK_VIEW_AUTO_REFRESH='true'`, `SettingsDefaultsManager.ts:787`). Every 10 minutes it checks for ungrouped conversations, due reviews, stale goals, or unplaced requests, and launches full agent runs (`src/services/agent-runner/run.ts:214`) using `haiku`/`sonnet` (`SettingsDefaultsManager.ts:793`). As the code comment explicitly states (`auto-refresh-worker.mjs:4-6`): *"Owner rule (Oct 22): AI magic, not speed. No minimum interval, no day window, no group cap, and a run is never stopped for taking long."* Users who never open the My Work UI still pay for these background agent runs out of their subscription.

### 3.5 Root Cause 4: Default Subscription Billing + Silent Quota Fallback onto Claude Pro/Max

- **Default config burns the user's Claude Code subscription**: `CLAUDE_MEM_PROVIDER='claude'` (`SettingsDefaultsManager.ts:587`), `CLAUDE_MEM_CLAUDE_AUTH_METHOD='subscription'` (`L606`), `CLAUDE_MEM_MODEL='haiku'` (`L551`). Even though Haiku is cheaper per token than Sonnet/Opus, running 10–30 observer turns + summaries + field condensations + My Work refreshes per coding session directly erodes the user's shared Claude Pro/Max 5-hour and 7-day rolling limits.
- **Silent Fallback onto Claude Subscription (`CLAUDE_MEM_QUOTA_FALLBACK_PROVIDER='claude'`)**: Even when a user explicitly configures `openrouter`, `gemini`, `cmem.ai`, or `openai-compatible` to stay off-plan, `SettingsDefaultsManager.ts:635` and `src/services/worker/provider-dispatch.ts:42-45` default `CLAUDE_MEM_QUOTA_FALLBACK_PROVIDER` to `'claude'` (and treat `''` as `'claude'`!). Any rate limit, timeout, 5xx outage, spent allowance, or 2× unusable reply on the off-plan provider **automatically diverts observation traffic onto the user's Claude Code subscription** unless the user explicitly sets `CLAUDE_MEM_QUOTA_FALLBACK_PROVIDER: "off"` (`docs/public/usage/quota-fallback.mdx:10,20,34-42`).
- **Late Precautionary Stop**: The subscription quota guard (`src/services/worker/RateLimitStore.ts:396-605`, `SettingsDefaultsManager.ts:638-642`) does not pause the observer until utilization hits **95%** of the 5-hour window, **93%** of the 7-day window, or **92%** of the 7-day Sonnet window — meaning the observer can consume almost the entire budget before stepping aside.

### 3.6 Root Cause 5: Token Inflation Inside the Primary Coding Session Itself

Even if the observer runs on a free external API, `claude-mem` inflates **every turn of the user's primary (Opus/Sonnet) coding session** via three injection channels:
1. **`SessionStart` Context Injection** (`src/cli/handlers/context.ts:670-731` → `src/services/context/ContextBuilder.ts:544-691` → `ContextBudget.ts:9`): Injects up to **`CONTEXT_OUTPUT_LIMIT = 10,000` chars (~2,500 tokens)** of recent observations (`CLAUDE_MEM_CONTEXT_OBSERVATIONS='50'`), session summaries (`CLAUDE_MEM_CONTEXT_SESSION_COUNT='10'`), last summary details, and open `work_state` entries on every `startup`, `/clear`, and `/compact`.
2. **`PreToolUse` Per-File Timeline Injection + File Read Gate** (`plugin/hooks/hooks.json:116-131`, `src/cli/handlers/file-context.ts:50,333-485`):
   - Fires synchronously on every `Read`, `Bash`, `Grep`, `Glob`, and `memory_search` call, appending prior observation timelines for up to 10 file paths via `hookSpecificOutput.additionalContext` into the primary model's active context.
   - With `CLAUDE_MEM_FILE_READ_GATE_ENABLED='true'` (`SettingsDefaultsManager.ts:666`), full-file `Read` calls on files **≥ 32 KiB** (`FILE_READ_GATE_DENY_MIN_BYTES = 32 * 1024`, `file-context.ts:50,333`) are **denied**, forcing the primary model to spend extra tool-calling round-trips calling `smart_outline` and `smart_unfold` (which saves tokens on huge files when it works, but adds round-trips and re-sends the entire primary conversation prefix each turn).
3. **Persistent MCP Tool Schema Footprint**: In worker mode, `mcp-server.ts` advertises **~17–20 MCP tools** by default (`important_workflow`, `search`, `timeline`, `get_observations`, `get_tool_uses`, `dumb_search`, `save_memory`, `session_start_context`, `work_state_write`, `work_state_read`, `smart_search`, `smart_outline`, `smart_unfold`, plus 6 `*_corpus` tools; `src/servers/mcp-server.ts:528-1185`, `mcp-tool-visibility.ts:25-75`). Every tool schema + multi-sentence prompt instruction sits in the primary coding model's context on every single API request.
4. *(Opt-in)* `CLAUDE_MEM_SEMANTIC_INJECT='false'` (`SettingsDefaultsManager.ts:697-699`, `src/cli/handlers/session-init.ts:232,369-383`): When enabled, injects the top 5 full observation narratives on **every `UserPromptSubmit`**, breaking prompt-cache stability and adding thousands of tokens per prompt.

---

## 4. OpenRouter & Multi-Provider System

### 4.1 Exact Role of OpenRouter (and What It Is NOT Used For)

In `claude-mem`, **OpenRouter is strictly an LLM inference transport for observer/summarization/compression/corpus text generation** — it is **NOT** used for embeddings, vector indexing, or search:

| Capability | Uses OpenRouter? | Actual Mechanism (`file:line`) |
|---|---|---|
| Observation extraction | **Yes** (when `CLAUDE_MEM_PROVIDER=openrouter` or `cmem.ai`) | `OpenRouterProvider` → `POST /chat/completions` (`OpenRouterProvider.ts:902-1360`) |
| Session summarization | **Yes** | `OpenAICompatibleProvider.processSummaryMessage()` (`OpenAICompatibleProvider.ts:672-709`) |
| Oversized field compression | **Yes** | `field-optimizer.ts` → `OpenRouterProvider.compressField()` (`OpenRouterProvider.ts:673`) |
| Hosted **`cmem.ai` Pro/Trial Observer** | **Yes — piggybacks on `OpenRouterProvider`!** | Sets `CLAUDE_MEM_PROVIDER='openrouter'` + `CLAUDE_MEM_OPENROUTER_BASE_URL='https://cmem.ai/...'` + `CLAUDE_MEM_PRO_MEMORY_KEY` (`src/shared/cmem-gateway.ts:4-50`, `OpenRouterProvider.ts:125-137,666`) |
| Optional My Work copy / refresh & Corpus Q&A | **Optional** | `CLAUDE_MEM_WORK_VIEW_COPY_PROVIDER`, `CLAUDE_MEM_CORPUS_AGENT='api'` (`SettingsDefaultsManager.ts:790,798`, `ApiCorpusAgent.ts:245`) |
| Embeddings | **No** | Local ONNX `all-MiniLM-L6-v2` via `uvx chroma-mcp` (worker) or `@huggingface/transformers` `LocalEmbedder` (server) (`ChromaSync.ts:711`, `LocalEmbedder.ts:31-40`) |
| Memory search (`search`/`timeline`) | **No** | Deterministic SQLite FTS5 (`unicode61`) + Chroma vector query / Postgres BM25 + `pgvector` (`SearchOrchestrator.ts:69-147`) |

### 4.2 Supported Providers, Defaults, Models & Auth Methods

Five selectable providers (`SELECTABLE_PROVIDERS` in `src/ui/provider-options.ts`, dispatched by `src/services/worker/provider-dispatch.ts:511`):

| Provider (`CLAUDE_MEM_PROVIDER`) | Implementation (`src/services/worker/`) | Auth Method(s) | Default Model | Batching Default | Notes |
|---|---|---|---|---|---|
| **`claude`** *(settings default)* | `ClaudeProvider.ts` (1,549 lines) | `subscription` (Claude Code OAuth/Keychain login, default) or `api-key` (`ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` in `~/.claude-mem/.env`, optional `ANTHROPIC_BASE_URL` for Bedrock/Vertex/LiteLLM gateways) | `haiku` (`CLAUDE_MEM_MODEL`) | `1` (`CLAUDE_MEM_CLAUDE_OBSERVATION_BATCH_SIZE`, range 1–32) | Spawns `claude` CLI via `@anthropic-ai/claude-agent-sdk`; `thinking: disabled` by default (`hardened-options.ts:229`); tier routing (`SIMPLE_TIER_TOOLS` → `haiku`, `tier-routing.ts:6-8`). |
| **`openrouter`** *(+ `cmem.ai` gateway)* | `OpenRouterProvider.ts` (1,448 lines) extends `OpenAICompatibleProvider.ts` | `CLAUDE_MEM_OPENROUTER_API_KEY` / `OPENROUTER_API_KEY` + multi-key pool `CLAUDE_MEM_OPENROUTER_API_KEYS` (up to 32 keys); or `cmem.ai` browser-paired `CLAUDE_MEM_PRO_MEMORY_KEY` | `cohere/north-mini-code:free` (`CLAUDE_MEM_OPENROUTER_MODEL`; supports JSON array fallback list sent as OpenRouter `models: [...]` at `OpenRouterProvider.ts:686`) | **None (1:1)** | Streams SSE (`stream: true` + `stream_options`) with 90s idle timeout (`CLAUDE_MEM_LLM_STREAM_IDLE_TIMEOUT_MS`); sends `reasoning: { enabled: false }` by default (`L683,707`), `usage: { include: true }` (`L718`), `HTTP-Referer`/`X-Title` attribution headers; auto-migrates retired free defaults (e.g. `xiaomi/mimo-v2-flash:free` → `cohere/north-mini-code:free`, `openrouter-provider.mdx:308`). Supports custom `CLAUDE_MEM_OPENROUTER_BASE_URL` (`openrouter-base-url.ts:37-74`) and `CLAUDE_MEM_OPENROUTER_EXTRA_BODY` (`L626`). |
| **`openai-compatible`** | `OpenAICompatProvider.ts` (879 lines) extends `OpenAICompatibleProvider.ts` | `CLAUDE_MEM_OPENAI_COMPAT_API_KEY` + multi-key pool `CLAUDE_MEM_OPENAI_COMPAT_API_KEYS` (optional for local Ollama/LM Studio/vLLM) | Per-preset default or empty (`CLAUDE_MEM_OPENAI_COMPAT_MODEL`) | `1` (`CLAUDE_MEM_OPENAI_COMPAT_OBSERVATION_BATCH_SIZE`, range 1–32) | Separated from `OpenRouterProvider` to avoid leaking OpenRouter headers/fields (`openai-compat-presets.ts:4-17`). Ships **27 built-in presets** (`nvidia-nim`, `deepseek`, `groq`, `together`, `ollama`, `lmstudio`, `vllm`, `opencode-go`, `opencode-zen`, `minimax`, `opper`, etc., `openai-compat-presets.ts:71-340`). Sends `reasoning: { enabled: false }` unless `CLAUDE_MEM_OPENAI_COMPAT_REASONING='on'`. |
| **`gemini`** | `GeminiProvider.ts` (701 lines) extends `OpenAICompatibleProvider.ts` | `CLAUDE_MEM_GEMINI_API_KEY` + multi-key pool `CLAUDE_MEM_GEMINI_API_KEYS` | `gemini-flash-latest` (`CLAUDE_MEM_GEMINI_MODEL`, via `v1beta/models/...:generateContent`) | **None (1:1)** | Built-in client-side free-tier RPM throttle (`CLAUDE_MEM_GEMINI_RATE_LIMITING_ENABLED='true'`, `GeminiProvider.ts:178-184,246-280`): `10 RPM` (`gemini-flash-latest`, `gemini-3.5-flash`), `15 RPM` (`gemini-flash-lite-latest`, `gemini-3.1-flash-lite`), `5 RPM` (`gemini-3-flash-preview`). |
| **`codex`** | `CodexProvider.ts` (492 lines) + `CodexAppServerClient.ts` (1,167 lines) + `CodexAppServerPool.ts` | Local `codex` CLI ChatGPT subscription login (`codex login`) | Codex CLI default (`CLAUDE_MEM_CODEX_MODEL=''`), reasoning effort `'low'` (`L601`) | **`8` events / `32,000` chars** (`SettingsDefaultsManager.ts:603-604`, `CodexProvider.ts:324-326`) | Pool of 2 `codex app-server` subprocesses (`CLAUDE_MEM_CODEX_MAX_CONCURRENT_AGENTS='2'`). Only provider with batching enabled out of the box. |

### 4.3 Cost-Control, Rate-Limit & Resilience Mechanisms

- **Per-Source Provider Routing**: `CLAUDE_MEM_PROVIDER_BY_SOURCE` (`SettingsDefaultsManager.ts:588`, `provider-dispatch.ts:15`) maps harnesses to different providers (e.g., `{"claude":"openrouter","codex":"codex"}`).
- **Multi-Key Rotation Pool** (`src/shared/api-key-pool.ts:58-107`, `docs/public/usage/multi-key-rotation.mdx`): Up to `MAX_POOL_KEYS = 32` keys for `gemini`, `openrouter`, and `openai-compatible`; rotates outside `withRetry` on `rate_limit`, `quota_exhausted` (30-min cooldown persisted in `api-key-cooldown.json`), or `auth_invalid`.
- **Paid-Send Budget Cap** (`src/services/worker/paid-send-budget.ts:11-61`): Caps total potentially billed sends per claimed batch at `DEFAULT_MAX_PAID_SENDS_PER_BATCH = 2` across retries, stalls, and output-drift re-asks.
- **Provider Quota Cooldown & Fallback** (`provider-dispatch.ts:59-511`, `docs/public/usage/quota-fallback.mdx`):
  - Cooldown windows: 30 minutes on `quota_exhausted`, 90 seconds on repeated unusable replies/outages, 15 minutes on `cmem.ai` gateway refusals (`CMEM_FALLBACK_RETRY_MS = 15 * 60_000`, `provider-dispatch.ts:24`).
  - Claude credential fallback (`CLAUDE_MEM_QUOTA_FALLBACK_AUTH_METHOD`: `subscription` ↔ `api-key` using `ANTHROPIC_FALLBACK_API_KEY`) tried before `CLAUDE_MEM_QUOTA_FALLBACK_PROVIDER`.
  - When pausing on quota/auth exhaustion, pending observations are **discarded** (not replayed later) to avoid backlog spend storms (`quota-fallback.mdx:54`).
- **Concurrency Caps**: `CLAUDE_MEM_MAX_CONCURRENT_AGENTS = '2'` (`SettingsDefaultsManager.ts:677`) for Claude SDK subprocesses; `CLAUDE_MEM_CODEX_MAX_CONCURRENT_AGENTS = '2'` (`L602`) for Codex.
- **Prompt Cache Optimization**: `observerInstructions(mode)` is placed strictly first before per-session variables (`src/sdk/prompts.ts:83-104`) and completed turns are kept immutable within a generation (`OpenAICompatibleProvider.ts:620-622`) to maximize provider prefix-cache hits.

*Docs references*: `https://docs.claude-mem.ai/usage/openrouter-provider`, `https://docs.claude-mem.ai/usage/gemini-provider`, `https://docs.claude-mem.ai/usage/openai-compatible-provider`, `https://docs.claude-mem.ai/usage/multi-key-rotation`, `https://docs.claude-mem.ai/usage/quota-fallback`.

---

## 5. Onboarding, Multi-Client Support & UI/Flow Friction

### 5.1 Onboarding & Installation Flow

Entry point: `npx claude-mem install` (`src/npx-cli/commands/install.ts`, **4,214 lines**; shell script `install/public/install.sh`).

1. **Heavy Runtime Dependency Chain**: Automatically checks and downloads/installs **Bun** (`ensureBun`, `install.ts:3446`) AND **`uv` / `uvx`** (`ensureUv`, `install.ts:3448`) so the worker can spawn Python `chroma-mcp` (`CLAUDE_MEM_PYTHON_VERSION='3.13'`, `SettingsDefaultsManager.ts:645`). Also copies plugin bundles to marketplace caches and installs a global `claude-mem` launcher shim (`ensure-launcher.cjs`).
2. **Browser Onboarding & Monetization Funnel (`src/npx-cli/install/browser-setup.ts:25-115`)**:
   - Interactive TTY installs without `--provider` launch the worker and open `http://127.0.0.1:<port>/?setup=1` (`setupPageUrl`, `browser-setup.ts:89-91`), polling `/api/setup/state` indefinitely while the user picks a storage/provider tier in the browser (steering toward **cmem.ai Pro / trial** OAuth device-code pairing and "On The Go" checkout, `install.ts:1934-2472`).
   - Unattended/agent installs print an explicit prompt injection directive (`AGENT_BROWSER_SETUP_HINT`, `browser-setup.ts:60-61`) asking the host agent to run `npx claude-mem install --setup browser` and open the setup URL.
   - Interactive terminal flows additionally prompt for Claude Auto-Memory disablement (`install.ts:315-334`), an email waitlist signup (`https://cmem.ai/api/waitlist`, `install.ts:2521-2650`), and PostHog telemetry opt-in (`install.ts:2675-2690`).
   - Headless installs without a delivered `cmem.ai` key fall back to `CLAUDE_MEM_PROVIDER='claude'` (`SettingsDefaultsManager.ts:583-587`), which silently bills the user's Claude Code subscription.

### 5.2 Multi-Client Support & Adapter Fragmentation

`claude-mem` already attempts broad multi-client coverage — **`src/npx-cli/commands/ide-detection.ts:57-194` detects 21 IDEs/agent harnesses** (`claude-code`, `opencode`, `openclaw`, `windsurf`, `codex-cli`, `t3code`, `kimi`, `devin`, `cursor`, `grok-bot`, `copilot-cli`, `copilot-vscode`, `antigravity`, `omp`, `hermes`, `pi`, `dsh`, `goose`, `roo-code`, `warp`, `zed`), implemented across **9 CLI hook adapters** (`src/cli/adapters/index.ts:1-28`) and **20+ bespoke installer modules** (`src/services/integrations/`):

| Client Tier | Clients | Integration Mechanism (`file:line`) | Concrete Limitations / Friction |
|---|---|---|---|
| **Tier 1: Native Plugin + Lifecycle Hooks + MCP** | **Claude Code**, **Codex CLI**, **Cursor** | Plugin manifests (`.claude-plugin/`, `.codex-plugin/`, `claude-mem-cursor/`) + hook JSONs (`plugin/hooks/hooks.json`, `codex-hooks.json`, `claude-mem-cursor/hooks/hooks.json`) + MCP server | Claude Code `Setup` hook requires a 1,000+ char POSIX shell one-liner (`hooks.json:14`); Cursor hooks invoke `npx -y claude-mem hook cursor ...` (`claude-mem-cursor/hooks/hooks.json:5-24`), adding CLI startup latency on every hook event; Codex requires TOML patching of `config.toml` + `codex-hooks.json` (28 KB!). |
| **Tier 2: Bespoke Hook/Extension Injection** | **Antigravity CLI (`agy`)**, **OpenCode**, **Windsurf**, **Gemini CLI**, **Devin**, **Kimi**, **OpenClaw**, **Hermes**, **OMP**, **DeepSeek Harness** | Hand-edited host config files (`AntigravityCliHooksInstaller.ts` 901 lines; `OpenCodeInstaller.ts` 491 lines + dual v1/v2 plugin API in `src/integrations/opencode-plugin/`; `WindsurfHooksInstaller.ts` 363 lines) | **Fragile host schema drift**: `docs/public/antigravity-cli/setup.mdx:138-149` documents 3 breaking regressions (`#4057` wrong Gemini event names in `~/.gemini/settings.json`; `#4911` `PreInvocation` injection breaking prompt caching; `#5078` `PreToolUse` returning `allow` bypassing `agy`'s permission prompts) AND admits writing MCP config to **two candidate paths** (`~/.gemini/antigravity/mcp_config.json` and `~/.gemini/config/mcp_config.json`) because it wasn't certain which one `agy` loads! Also only targets `agy` CLI, not Antigravity Desktop IDE (`setup.mdx:17`). |
| **Tier 3: MCP-Only / Markdown Rules / Transcript Watch** | **Zed**, **Goose**, **Warp**, **Copilot CLI / VS Code**, **Roo Code**, **Pi**, **Aider**, **T3 Code**, **Grok Bot** | `McpIntegrations.ts` (771 lines) writing JSON/JSONC/YAML MCP entries + static `AGENTS.md`/`GEMINI.md`/`CLAUDE.md` markers, or transcript file watching (`CLAUDE_MEM_TRANSCRIPTS_ENABLED`) | No automatic `PostToolUse` capture (manual recall only, e.g., Pi hint `"manual recall; check automatic capture compatibility"` at `ide-detection.ts:160`), or delayed transcript-tail polling instead of live hooks. |

Every client writes observations into the same SQLite DB discriminated by `sdk_sessions.platform_source` (`SessionStore.ts:131,577`), with `CLAUDE_MEM_SESSION_START_INCLUDE_ALL_SOURCES` defaulting to **`'false'`** (`SettingsDefaultsManager.ts:553`) — meaning **by default, memories captured in Cursor/Codex/Antigravity are NOT surfaced in Claude Code's `SessionStart` context (and vice versa)**!

### 5.3 Concrete Web UI & Hook Pipeline UX Weaknesses

1. **Split-Brain Web UI (`/` vs `/viewer`)**: Visiting the worker root URL opens **My Work** (`ViewerRoutes.ts:234`, a 246-file Vite/Tailwind/shadcn project-management app with particles/artwork and background agent refreshes), NOT the memory timeline! The actual memory database viewer is hidden at `/viewer` (`ViewerRoutes.ts:235`) as a completely separate esbuild React bundle with different styling, state management, and UX conventions.
2. **Viewer Search Does NOT Use FTS5 or Vector Search**: In `/viewer` (`src/ui/viewer/App.tsx:313-315` → `src/ui/viewer/utils/feed-display.ts:119-132`), typing into the timeline search box only runs `matchesSearch(item, timelineQuery)` — an in-memory **case-insensitive word-substring filter over already-paginated rows**! If a memory is older than the currently scrolled pages (or past `DataRoutes`'s hard `10,000`-row pagination cap documented in `docs/public/architecture/worker-service.mdx:127`), the viewer search says `"No observations to show"` and forces manual `"Load older memories"` clicks instead of querying `/api/search`.
3. **Configuration Overload (252 Settings Keys)**: `SettingsDefaultsManager.ts` defines **252 distinct `CLAUDE_MEM_*` settings keys**. `ContextSettingsModal.tsx` (1,099 lines) exposes only a subset across multi-tabbed panels, many settings silently require a manual worker restart (`ContextSettingsModal.tsx:261,628`), and security-sensitive settings (`CLAUDE_MEM_OPENROUTER_BASE_URL`, `CLAUDE_MEM_OPENROUTER_EXTRA_BODY`, `CLAUDE_CODE_PATH`, `CLAUDE_MEM_NOTION_TOKEN`) are rejected by `POST /api/settings` and force manual edits to `~/.claude-mem/settings.json` (`ContextSettingsModal.tsx:976`).
4. **Intrusive `PreToolUse` File Read Gate**: Enabled by default (`CLAUDE_MEM_FILE_READ_GATE_ENABLED='true'`, `SettingsDefaultsManager.ts:666`), synchronously intercepts `Read` calls and **denies** reading files ≥ 32 KiB (`file-context.ts:50,333-349`), hijacking the agent's native file-reading flow to force `smart_outline`/`smart_unfold` MCP calls — adding synchronous latency (up to `3,000ms` budget per read, `file-context.ts:60`) and surprise denials.
5. **Runtime Fragility & Operational Complexity**:
   - Requires **three runtimes** simultaneously for the default worker stack: **Node.js** (MCP server & `ensure-launcher.cjs`), **Bun** (worker daemon & `bun:sqlite`), and **Python / `uvx`** (`chroma-mcp==0.2.6` + ONNX runtime). `ChromaMcpManager.ts` is **2,823 lines** largely dedicated to working around `uvx` PATH resolution, Python env pollution, hardlink exhaustion, IPv6 URL bugs, CPU core pinning (`#6365`), 60s cold-cache timeouts (`#4682`), and orphan process sweeping (`src/supervisor/orphan-chroma-sweep.ts`).
   - Documentation drift: Even the official `docs/public/usage/getting-started.mdx:136-150` gives raw SQLite example queries referencing removed column names (`session_id`, `observations.tool_name`) that fail against `v16.1.2`.

---

## 6. Key Design Takeaways for `0x1-ai-memory`

1. **Never fire 1 LLM call per `PostToolUse` (and never replay $O(N^2)$ raw tool histories)**: Capture tool events cheaply into SQLite (`tool_uses`/event log) with zero LLM calls on the hot path; distill in **bounded, stateless single-shot batches** (at turn/stop boundaries or N-event/token thresholds) with strict deterministic pre-filtering (`Read`/`Glob`/`Grep`/`ls` noise collapsed before any LLM sees them), and **never spawn a secondary LLM call just to condense a large file read** (smart deterministic head/tail + diff extraction beats paying twice).
2. **Never silently bill the user's primary coding subscription**: Make the memory model choice explicit, off-plan by default (local Ollama / OpenRouter cheap or free tier / user API key / deterministic zero-LLM mode), with **zero silent fallback** onto Claude/ChatGPT subscriptions and **zero unrequested background cron agents** (unlike My Work's 10-minute `AutoRefreshWorker`).
3. **Keep the primary coding session context razor-thin**: Replace 10 KB `SessionStart` dumps, intrusive `PreToolUse` file-read gates, and 20-tool MCP bloat with a **compact ~3–5 tool MCP surface** (`search`, `timeline`, `get`, `save`) and a small, high-signal startup index (~1–2 KB).
4. **Eliminate the 3-runtime stack (Node + Bun + Python/`uvx` Chroma)**: Run a **single self-contained runtime/binary** using SQLite + FTS5 + native embedded vectors (e.g., `sqlite-vec` or in-process quantized vectors) with zero Python/Chroma sidecars.
5. **Cross-agent memory by default, unified via a clean adapter contract**: Store a normalized `source_agent` tag, default to **cross-agent project recall enabled** (solving the main reason users want a cross-agent memory system), and back the Web UI search directly with the real FTS5 + vector search API rather than client-side page filtering.
