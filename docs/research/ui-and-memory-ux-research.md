# Primary-Source Research: UI/UX, Visualization & Memory Engine Improvements for `0x1-ai-memory`

> **Baseline audited:** `0x1-ai-memory` `v0.2.3` — single Rust binary (`Cargo.toml`: `rusqlite 0.38`, `sqlite-vec 0.1.6`, `fastembed 7.8`, `rust-embed 8`), embedded React `19.0.0` + Vite `6.1.0` SPA (`ui/package.json` had zero runtime UI libraries; built `ui/dist/assets/index-*.js` was **292.2 KB raw / 85.6 KB gzipped** and CSS was **25.2 KB raw / 4.9 KB gzipped**).
> **Existing schema & routes leveraged:** `projects`, `sessions`, `raw_events`, `memories`, `memories_fts`, `memories_vec` (`float[384] distance_metric=cosine`), `usage_ledger` (`src/db.rs`, `src/models.rs`), plus `/api/memories/{id}/timeline` and `/api/usage` (`src/server.rs`).

---

## Executive Summary: High-Signal Improvements vs Low-ROI Traps

| Initiative | Category | Signal / ROI | Why (Primary Evidence) |
| :--- | :--- | :--- | :--- |
| **Local Neighborhood ("Local Graph") Drawer / Inspector** (1–2 hop depth, filterable by edge type) | Graph / Explore | **High-Signal** | Obsidian Local Graph + van Ham & Perer (IEEE TVCG 2009) show *"Search, Show Context, Expand on Demand"* avoids hairball overload for 10–1,000 nodes while surfacing concrete file/session/supersession links. |
| **Zero-LLM Derived Edges** (shared `files`, shared `tags`, same `session_id`, `superseded_by` DAG, top-k `memories_vec` cosine neighbors) | Engine + Graph | **High-Signal** | Zero extra LLM spend (preserves anti-drain invariant); directly mirrors Mem0 OpenMemory's SQL-join `/related` endpoint and adds semantic + lineage edges using data already in SQLite. |
| **Multi-Axis Zero-LLM Grouping on `/memories`** (Project, Session Timeline, File/Module Prefix Tree, Supersession Chain, Type/Tag) | UI / Grouping | **High-Signal** | Engineers debug by repo/file/session, not flat chronological feeds. Every required column (`project_id`, `session_id`, `files_json`, `tags_json`, `memory_type`, `superseded_by`) already exists in `src/db.rs`. |
| **Two-Layer Live Stream** (distilled memories + toggleable live `raw_events` / distillation pipeline tail & session activity bar) | Live Stream | **High-Signal** | Previously `ui/src/App.tsx` listened to SSE `raw_event` only to refetch `memories`, hiding queued events, errors, and distillation status. Mirrors Langfuse/Phoenix trace timelines & Letta ADE live step visibility. |
| **Context Window / Briefing Inspector + Recall-Hit Telemetry** | Observability | **High-Signal** | Letta ADE's right-panel Context Window Viewer + Mem0 OpenMemory's `MemoryAccessLog` make memory ROI verifiable (which client read which memory, token budget used vs raw events compressed). |
| **Custom Canvas 2D / SVG + `d3-force` (`5.7 KB` gzip) or pure deterministic DAG/radial layout (`0 KB`)** | Frontend Viz | **High-Signal** | Keeps embedded binary tiny (+6.7% over 85.6 KB gzipped JS), 100% React 19 native, matches Rim dark-console aesthetics, and handles 10–1,000 nodes cleanly without WebGL or DOM bloat. |
| **Global Unfiltered Force-Directed "Hairball" Graph as Primary View** | Graph UI | **Low-ROI Trap** | At <500 sparse engineering memories, a global graph either fractures into disconnected islands (orphans) or collapses into unreadable hairballs when dense cosine edges are added (Shneiderman 1996; van Ham & Perer 2009; Obsidian Help). |
| **LLM-Extracted Entity-Relation Knowledge Graph per Turn** (Graphiti / Mem0 Graph style) | Engine | **Low-ROI Trap** | Violates `0x1-ai-memory`'s anti-drain invariant: Zep/Graphiti runs **6 LLM stages per episode** (`help.getzep.com/v4/how-graph-creation-works`); Mem0 Graph Memory adds **3 extra LLM calls per `add()`** (`mem0@v1.0.0`, `graph_memory.py:195-345`). |
| **Heavy Graph/Diagram Dependencies (`cytoscape` 135.6 KB gz, `sigma`+`graphology` 110.1 KB gz, `@xyflow/react` 86.3 KB gz)** | Frontend Viz | **Low-ROI Trap** | Each single dependency equals or exceeds the entire React 19 app bundle (`85.6 KB` gz) and adds either WebGL fragility (`sigma`), DOM scaling limits + separate layout dep (`@xyflow/react`), or legacy React wrapper issues (`react-cytoscapejs`). |
| **Split-Brain Second UI / Autonomous Background "My Work" Agent** (`claude-mem` pattern) | UI / Engine | **Low-ROI Trap** | `claude-mem` v16.1.2's separate 246-file My Work SPA + `/viewer` split confused search (client-side substring filtering only) and added unrequested background LLM cron work (`docs/research/claude-mem-analysis.md:93-95,327-328`). |

---

## 1. AI Agent Memory & Dev Console UI Patterns

### 1.1 Letta (MemGPT) ADE — Context Transparency & Memory Tier Separation
- **Primary sources:**
  - Letta ADE overview: https://docs.letta.com/v1-sdk/ade
  - Memory blocks (core memory): https://docs.letta.com/v1-sdk/memory/memory-blocks
  - Archival memory: https://docs.letta.com/v1-sdk/memory/archival-memory
  - Context hierarchy: https://docs.letta.com/v1-sdk/memory/context-hierarchy
- **Observed UI & architecture patterns:**
  - **Three-panel mental model** (`docs.letta.com/v1-sdk/ade`): Left panel = Agent Configuration (model, system instructions, tools, context limits); Center panel = Agent Simulator (live conversation + real-time tool usage & reasoning stream); Right panel = **Agent State Visualization** containing three distinct inspectors:
    1. **Context Window Viewer** — exact view of what tokens/blocks are currently loaded in the prompt.
    2. **Core Memory Blocks** — always-in-context, editable XML-framed sections with explicit character budgets (`chars_current` / `chars_limit`, recommended `<50k` chars across `<20` blocks).
    3. **Archival Memory** — out-of-context vector database (`archival_memory_search`, `archival_memory_insert`), unlimited count, ~300-token recommended passage size, organized by tags, queried on demand.
- **Actionable lessons for `0x1-ai-memory`:**
  - `0x1-ai-memory` already mirrors this two-tier split cleanly: **Pinned memories + `SessionStart` briefing (`session_start_token_budget` = 450 tokens)** ≈ Letta's always-in-context Core Memory Blocks; **full SQLite FTS5 + `sqlite-vec` store (`memory_search`/`memory_get`)** ≈ Letta's Archival Memory.
  - Elevate the existing `Briefing preview` modal into an inspectable **Context Budget Breakdown** showing exact token allocation by tier (Pinned vs Recent Project Headlines vs Global Preferences vs remaining headroom against `session_start_token_budget`) with 1-click unpin right from the preview.

### 1.2 Mem0 / OpenMemory — Cross-App Governance, Categories & Access Logs
- **Primary sources:**
  - OpenMemory MCP data model & UI (`mem0ai/mem0@v1.0.0`, later archived into `mem0ai/openmemory/openmemory-archive`):
    - Schema: `https://github.com/mem0ai/mem0/blob/v1.0.0/openmemory/api/app/models.py` (L30–L185)
    - Related memories endpoint: `https://github.com/mem0ai/mem0/blob/v1.0.0/openmemory/api/app/routers/memories.py` (`GET /{memory_id}/related`)
    - UI components: `https://github.com/mem0ai/mem0/tree/v1.0.0/openmemory/ui/app` (`memories/components/FilterComponent.tsx`, `memory/[id]/components/AccessLog.tsx`, `RelatedMemories.tsx`, `apps/components/AppGrid.tsx`)
  - Mem0 Platform Graph Memory docs: https://docs.mem0.ai/platform/features/graph-memory
  - Mem0 OSS Graph Memory implementation: `https://github.com/mem0ai/mem0/blob/v1.0.0/mem0/memory/graph_memory.py` (L75–L345)
  - Mem0 paper: *Mem0: Building Production-Ready AI Agents with Scalable Long-Term Memory* (https://arxiv.org/abs/2504.19413)
- **Observed UI & architecture patterns:**
  - **Lifecycle state audit trail (`MemoryStatusHistory`, `models.py:161-167`):** Tracks `old_state -> new_state` (`active | paused | archived | deleted`) and `changed_by`.
  - **Cross-app access logs (`MemoryAccessLog`, `models.py:176-183`, rendered by `AccessLog.tsx`):** Records `(memory_id, app_id, access_type, accessed_at)` so users can open any memory and see *which AI client* actually read/recalled it and when.
  - **App-centric view (`apps/components/AppGrid.tsx`):** Shows per-client counts of memories created vs accessed, paired with per-app access controls (`AccessControl`, `models.py:132`).
  - **Zero-LLM Related Memories (`memories.py`, `GET /{memory_id}/related`):** OpenMemory's detail page `RelatedMemories.tsx` does **not** run a graph DB or LLM call—it performs a direct SQL join on shared `categories` (`Memory.categories`).
  - **Where Mem0 adds heavy cost:** Mem0's hosted/OSS Graph Memory (`graph_memory.py:195-345`) extracts entities and relations via **3 separate `self.llm.generate_response()` calls** (`_retrieve_nodes_from_data` L200, `_establish_nodes_relations_from_data` L257, `_get_delete_entities_from_search_output` L340) *on top of* vector memory extraction (`mem0/memory/main.py:958`).
- **Actionable lessons for `0x1-ai-memory`:**
  - Adopting a lightweight `memory_recall_stats` table (incrementing `recall_count`, `last_recalled_at`, and `clients_json` when `SessionStart`, `UserPromptSubmit`/`PreInvocation`, `memory_search`, or `memory_get` serves a memory) unlocks high-signal trust telemetry ("recalled 14× across Codex & Claude Code") at zero LLM cost.

### 1.3 Zep / Graphiti — Bi-Temporal Supersession & Source Traceability
- **Primary sources:**
  - Zep paper: *Zep: A Temporal Knowledge Graph Architecture for Agent Memory* (https://arxiv.org/abs/2501.13956)
  - Graphiti source:
    - Bi-temporal edges: `https://github.com/getzep/graphiti/blob/main/graphiti_core/edges.py` (`EntityEdge` L228–L255: `created_at`, `expired_at`, `valid_at`, `invalid_at`, `episodes: list[str]`, `fact`)
    - Node hierarchy: `https://github.com/getzep/graphiti/blob/main/graphiti_core/nodes.py` (`EpisodicNode` L245, `EntityNode` L499, `CommunityNode` L787, `SagaNode` L967)
    - Ingestion pipeline: `https://github.com/getzep/graphiti/blob/main/graphiti_core/graphiti.py` (`add_episode` L1815–L1906)
  - Zep v4 official docs:
    - Facts & bi-temporal validity: https://help.getzep.com/v4/facts
    - How Graph Creation Works: https://help.getzep.com/v4/how-graph-creation-works
    - Source traceability: https://help.getzep.com/v4/source-traceability
    - Graph visualizer assistant: https://help.getzep.com/v4/graph-visualizer-assistant
- **Observed UI & architecture patterns:**
  - **Non-destructive invalidation (supersession):** Rather than deleting outdated facts, Graphiti sets `invalid_at` / `expired_at` on superseded edges (`edges.py:236-250`). Active queries exclude invalidated facts by default while preserving a full temporal audit trail.
  - **Source traceability (`source-traceability.md`, `EpisodicNode`):** Every derived artifact links back to the raw `episodes` that produced it, enabling developers to verify *why* a fact exists.
  - **Why Graphiti's full LLM graph ingestion does NOT fit `0x1-ai-memory`:** Zep's official pipeline table (`help.getzep.com/v4/how-graph-creation-works`) explicitly documents **6 LLM-powered steps per episode**: (1) Extract entities, (2) Extract relationships/facts, (3) Date facts (`valid_at`/`invalid_at`), (4) Resolve entities, (5) Resolve/invalidate contradicting relationships, (6) Summarize entities — plus optional community summarization (`graphiti.py:1900`). It also requires a graph DB backend (Neo4j, FalkorDB, or Neptune).
- **Actionable lessons for `0x1-ai-memory`:**
  - `0x1-ai-memory` **already has** the two highest-value Graphiti primitives in SQLite with zero graph DB overhead:
    1. **Non-destructive supersession:** `memories.status = 'superseded'` + `memories.superseded_by` (`src/models.rs:54-55`, `src/db.rs:638-645`).
    2. **Episode provenance:** `memories.session_id` → `sessions` → `raw_events` (`src/models.rs:28-46`).
  - Visualizing **Supersession Chains (`v1 → v2 → current`)** and **Source Session / Raw Event Provenance** delivers 80% of Graphiti's debugging value at **0 extra LLM tokens**.

### 1.4 `claude-mem` — Progressive Disclosure Wins vs Web Viewer Pitfalls
- **Primary sources:**
  - Official docs: https://docs.claude-mem.ai/progressive-disclosure, https://docs.claude-mem.ai/usage/my-work, https://docs.claude-mem.ai/architecture/overview
  - Repo primary-source audit of `thedotmack/claude-mem` v16.1.2: `docs/research/claude-mem-analysis.md` (Sections 1.3, 1.4, 5.3)
- **What works well:**
  - **3-Layer Progressive Disclosure (`progressive-disclosure.mdx`):** Index (~50–100 tok/row) → chronological Timeline (~100–200 tok/row) → full Details on demand, cutting prompt bloat by ~10× vs dumping raw history (`35,000` raw tokens → `~920` tokens for index + 1 fetch).
  - **Token-size / savings framing:** Displaying compact token cost hints and cumulative briefing efficiency reinforces *why* progressive disclosure matters.
  - **Session/timeline anchor view (`/api/timeline`):** Lets engineers inspect what happened right before and after an observation.
- **What creates UI clutter & traps to avoid (`docs/research/claude-mem-analysis.md:93-95,327-329`):**
  - **Split-brain UI (`/` "My Work" vs `/viewer`):** Maintaining two separate React SPAs fragments mental models and bloats assets.
  - **Client-side-only search:** `/viewer` filtered only paginated rows in memory rather than hitting FTS5/vector search. Keep all views backed by SQLite queries.
  - **Settings sprawl:** 252 `CLAUDE_MEM_*` keys vs `0x1-ai-memory`'s disciplined ~12 settings.

### 1.5 Obsidian Graph / Local Graph & Developer Trace UIs (Langfuse, Arize Phoenix)
- **Primary sources:**
  - Obsidian Graph View & Local Graph official docs: https://obsidian.md/help/plugins/graph (source markdown: `https://github.com/obsidianmd/obsidian-help/blob/master/en/Plugins/Graph%20view.md`)
  - Langfuse Agent Graphs & Trace Detail View: https://langfuse.com/docs/observability/features/agent-graphs, https://langfuse.com/docs/observability/overview
  - Arize Phoenix Tracing & Span Trees: `https://github.com/Arize-ai/phoenix/blob/main/docs/phoenix/tracing/features-tracing.mdx`, `docs/phoenix/tracing/concepts-tracing/what-are-traces.mdx`
  - Foundational Visualization HCI papers:
    - Ben Shneiderman (1996), *"The Eyes Have It: A Task by Data Type Taxonomy for Information Visualizations"*, IEEE Symposium on Visual Languages (DOI: https://doi.org/10.1109/VL.1996.545307)
    - Frank van Ham & Adam Perer (2009), *"Search, Show Context, Expand on Demand: Supporting Large Graph Exploration with Degree-of-Interest"*, IEEE TVCG (DOI: https://doi.org/10.1109/TVCG.2009.108)
- **Why global hairball graphs fail for <500 engineering memories:**
  1. **Edge density bimodal failure:** With only explicit links (`superseded_by`, shared files), early memory stores (<200 memories) look like sparse confetti with many orphans. Adding loose global semantic similarity edges connects everything into an unreadable hairball.
  2. **No reading order:** Engineering debugging is temporal and causal. Force-directed global layouts destroy chronological order unless paired with timeline/lineage views.
- **What makes visual maps genuinely actionable for engineers:**
  - **Obsidian Local Graph controls (`Graph view.md`):** Anchored on the selected memory; **Depth control**; **Orphans toggle**; **Color-coded Groups** (`memory_type` or `source_client`); **Direction arrows** (for `superseded_by`).
  - **Langfuse's multi-view pattern (`agent-graphs`):** Toggle cleanly between **List/Grouped**, **Timeline/Lineage**, and **Graph (Aggregated vs Expanded)** on the *same* underlying data. In Aggregated mode, repeating steps collapse (e.g., `edit_file × 4`) instead of flooding the view.
  - **Arize Phoenix's Span Waterfall (`features-tracing.mdx`, `what-are-traces.mdx`):** Pairing a live activity/session waterfall alongside a structured detail inspector makes multi-step pipelines (hook capture → pending raw events → LLM distillation → emitted memory) immediately diagnosable.

---

## 2. Lightweight Visualization & Graph Libraries for an Embedded Rust Binary SPA

`ui/dist/` is embedded directly into the `0x1-memory` binary via `rust-embed` (`src/server.rs`). Baseline `v0.2.3` built bundle (`ui/dist/assets/`):
- `index-*.js`: **292.2 KB raw / 85.6 KB gzipped** (`react` + `react-dom` 19 + app)
- `index-*.css`: **25.2 KB raw / 4.9 KB gzipped**

### Primary-Source Package Comparison (Verified via `registry.npmjs.org` & `bundlephobia.com`)

| Option | Latest Version & License | Minified / Gzipped Size | Direct Deps | React 19 Compatibility | Renderer & Interactivity | Best Fit for 10–1,000 Nodes in `0x1-ai-memory` |
| :--- | :--- | :--- | :---: | :--- | :--- | :--- |
| **a1) Pure Custom SVG / Canvas 2D** (deterministic radial/DAG/timeline layout, 0 deps) | N/A (in-repo) | **~3–6 KB / ~1.5–2.5 KB** | 0 | **Native (100%)** | SVG (≤150 nodes: crisp text, CSS Rim tokens, keyboard/ARIA focus) or Canvas 2D (≤1,000 nodes) | **Best for Local Neighborhood (Depth 1), Supersession DAGs, and Session Timelines.** |
| **a2) Custom Canvas 2D / SVG + `d3-force`** | `d3-force@3.0.1` (ISC) | **15.3 KB / 5.7 KB** | 3 (`d3-dispatch`, `d3-quadtree`, `d3-timer`) | **100%** | Velocity Verlet physics (`forceManyBody`, `forceLink`, `forceCollide`). Pair with Canvas 2D (`devicePixelRatio` aware). | **Best overall for interactive cluster/map view.** Adds only **5.7 KB gzip** (+6.7% of JS). Pre-tick `sim.tick(120)` before paint to avoid jitter. |
| **b) `@xyflow/react`** (React Flow) | `@xyflow/react@13.0.3` (MIT) | **255.6 KB / 86.3 KB** *(excludes layout lib)* | 4 | **Yes** | HTML DOM nodes + SVG edges; no built-in auto-layout (`reactflow.dev/learn/layouting/layouting`). | **Overkill.** Doubles JS bundle (+101% gz), still needs `dagre`/`d3-force`. |
| **c) `cytoscape`** | `cytoscape@3.35.0` (MIT) | **431.5 KB / 135.6 KB** | 0 | **Manual `useRef` only** | Canvas 2D + full graph algorithm suite. | **Too heavy (+158% gz).** Graph topology computation belongs in Rust/SQLite, not a 431 KB JS bundle. |
| **d) `sigma` + `graphology`** | `sigma@4.0.6` + `graphology@0.26.0` + `forceatlas2@0.10.1` (MIT) | **441.5 KB / 110.1 KB** | 4 combined | **Manual `useRef` only** | WebGL2 renderer designed for 10,000–100,000+ nodes. | **Wrong scale & unnecessary WebGL risk (+129% gz).** |
| **e) `force-graph` / `react-force-graph-2d`** | `force-graph@1.52.0` / `react-force-graph-2d@1.29.2` (MIT) | **168.9 / 56.0 KB** or **177.0 / 58.4 KB** | 15 | **Works with caveat** (`react-kapsule` + `prop-types`) | Canvas 2D, batteries-included. | **10× heavier than `d3-force` alone** (58.4 KB vs 5.7 KB gz). |

---

## 3. High-Leverage Memory Engine, Grouping & Live Stream Improvements

### 3.1 Zero-LLM Grouping Strategies (100% Derived from Existing Schema)
1. **By Project & Scope (`scope`, `project_id`, `projects.privacy_mode`)**
2. **By Session / Chronological Episode (`sessions` + `memories.session_id`)** + wired `GET /api/memories/{id}/timeline`
3. **By File Path (`memories.files_json`)**
4. **By Supersession Lineage (`memories.superseded_by`, `status`)**
5. **By Tags & Memory Type (`tags_json`, `memory_type`)**
6. **Semantic Clusters via `sqlite-vec` (`memories_vec`, 384-dim cosine):**
   - Primary source: https://alexgarcia.xyz/sqlite-vec/features/knn.html, https://alexgarcia.xyz/sqlite-vec/features/vec0.html, https://github.com/asg017/sqlite-vec/releases.
   - Note on pinned `sqlite-vec = "0.1.6"` (`Cargo.toml:28`): `v0.1.6` lacks `distance <= ?` constraints inside `vec0` KNN queries (introduced in `0.1.7`), so filter cosine similarity ($\ge 0.72$) cleanly in Rust when computing graph edges.

### 3.2 Live Stream & Observability Upgrades
1. **Live Pipeline Activity Tail** (`GET /api/raw-events` + SSE `raw_event` and `usage_recorded` events, aggregated consecutively like Langfuse Aggregated mode).
2. **Active Sessions Bar** (`GET /api/sessions` + 1-click `POST /api/sessions/{id}/distill` and `POST /api/distill/flush`).
3. **Recall-Hit & Token-Savings Telemetry** (`memory_recall_stats` incremented on `SessionStart`, `UserPromptSubmit`/`PreInvocation`, `memory_search`, and `memory_get`; plus Context Budget Breakdown showing briefing tokens vs full project archive tokens).

### 3.3 Graph Edges: Zero-LLM Derived Edges vs LLM Entity-Relation Triples

| Dimension | Zero-LLM Derived Edges (Adopted) | LLM-Extracted Entity-Relation Triples (Rejected) |
| :--- | :--- | :--- |
| **LLM calls per turn/episode** | **0 calls / $0.00** (works in Zero-LLM safe mode & `local_only`) | **3–6+ LLM calls per episode** (`mem0@v1.0.0` `graph_memory.py`: 3 calls; Zep/Graphiti: 6 stages) |
| **Edge types** | 1. `supersedes` (directed `superseded_by`)<br>2. `same_file` (hub-damped shared paths)<br>3. `same_session` (chronological within session)<br>4. `shared_tag` (hub-damped shared tags)<br>5. `semantic_similar` (top-3 cosine $\ge 0.72$) | `(Subject) --[PREDICATE]--> (Object)` |
| **Anti-hairball rules** | Hub damping (>30% project frequency for files / >35% for tags suppressed); per-edge-type toggles (`shared_tag` off by default); Local 1-hop SVG in inspector drawer | Often produces noisy disconnected fragments or dense entity hubs without manual pruning |

---

## Primary Sources Index
1. **Letta (MemGPT):** https://docs.letta.com/v1-sdk/ade, https://docs.letta.com/v1-sdk/memory/memory-blocks, https://docs.letta.com/v1-sdk/memory/archival-memory, https://docs.letta.com/v1-sdk/memory/context-hierarchy
2. **Mem0 & OpenMemory:** https://github.com/mem0ai/mem0/blob/v1.0.0/openmemory/api/app/models.py, https://github.com/mem0ai/mem0/blob/v1.0.0/openmemory/api/app/routers/memories.py, https://github.com/mem0ai/mem0/tree/v1.0.0/openmemory/ui/app, https://github.com/mem0ai/mem0/blob/v1.0.0/mem0/memory/graph_memory.py, https://docs.mem0.ai/platform/features/graph-memory, https://arxiv.org/abs/2504.19413
3. **Zep & Graphiti:** https://arxiv.org/abs/2501.13956, https://github.com/getzep/graphiti/blob/main/graphiti_core/edges.py, https://github.com/getzep/graphiti/blob/main/graphiti_core/nodes.py, https://github.com/getzep/graphiti/blob/main/graphiti_core/graphiti.py, https://help.getzep.com/v4/facts, https://help.getzep.com/v4/how-graph-creation-works, https://help.getzep.com/v4/source-traceability
4. **`claude-mem`:** https://docs.claude-mem.ai/progressive-disclosure, https://docs.claude-mem.ai/usage/my-work, https://docs.claude-mem.ai/architecture/overview, `docs/research/claude-mem-analysis.md`
5. **Obsidian, Trace UIs & HCI Literature:** https://obsidian.md/help/plugins/graph, https://langfuse.com/docs/observability/features/agent-graphs, https://langfuse.com/docs/observability/overview, https://github.com/Arize-ai/phoenix/blob/main/docs/phoenix/tracing/features-tracing.mdx, https://doi.org/10.1109/VL.1996.545307, https://doi.org/10.1109/TVCG.2009.108
6. **Visualization & `sqlite-vec` Specs:** https://github.com/d3/d3-force, https://reactflow.dev/learn/advanced-use/performance, https://github.com/cytoscape/cytoscape.js, https://github.com/jacomyal/sigma.js, https://github.com/vasturiano/force-graph, https://alexgarcia.xyz/sqlite-vec/features/knn.html, https://alexgarcia.xyz/sqlite-vec/features/vec0.html
