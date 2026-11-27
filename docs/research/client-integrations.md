# Multi-Client AI Coding Integration Research & T3 Code Reverse Engineering

> **Scope & Provenance:** Primary-source technical investigation conducted on macOS (`darwin/arm64`) against locally installed binaries, configuration schemas, official documentation, and shipped application artifacts. Zero credentials, API keys, OAuth tokens, cookies, or private conversation contents were read or included.
>
> **Observed Local Versions:**
> - **Claude Code CLI:** `2.1.331` (`~/.local/bin/claude`)
> - **Codex CLI:** `codex-cli 0.172.0` (`~/.codex/packages/standalone/releases/0.172.0-aarch64-apple-darwin/bin/codex`)
> - **Antigravity:** `/Applications/Antigravity.app` & `~/.local/bin/agy` (docs from `~/.gemini/antigravity/builtin/skills/agy-customizations/`)
> - **T3 Code (Beta):** `0.1.11` (`/Applications/T3 Code (Beta).app/Contents/Resources/app.asar`, `~/.t3/userdata/statev2.sqlite`)

---

## Executive Summary & Key Findings

1. **All three native agent harnesses (Claude Code, Codex CLI, Antigravity) now have full lifecycle hooks AND Streamable HTTP + stdio MCP support**, enabling both **push-based memory injection** (hook context injection at session/prompt start) and **pull-based memory retrieval/writes** (MCP tools).
2. **Codex CLI (`0.172.0`) has matured far beyond legacy `notify`:** it ships a full Claude-compatible `~/.codex/hooks.json` engine (14 lifecycle events including `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PreCompact`, `Stop`, `SessionEnd`, `SubagentStart`) supporting `hookSpecificOutput.additionalContext`, gated by a **cryptographic SHA-256 hook trust model** (`[hooks.state]` in `~/.codex/config.toml` + `~/.codex/.hook-trust/`).
3. **Antigravity has a distinct hook protocol:** camelCase protojson payloads, 6 events (`SessionStart`, `PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation`, `Stop`), and context injection via `injectSteps: [{ systemMessage: { systemMessage: "..." } }]` instead of `hookSpecificOutput.additionalContext`. Its global MCP config lives at `~/.gemini/config/mcp_config.json` (with `~/.gemini/antigravity/mcp_config.json` symlinked to it) and accepts **stdio** and **Streamable HTTP** (not legacy SSE).
4. **T3 Code (`0.1.11`) reverse-engineering bombshell:**
   - **Claude Code under T3 Code:** Runs via bundled `@anthropic-ai/claude-agent-sdk` (`query()`) **without** `--setting-sources`, `--strict-mcp-config`, or `disableAllHooks` on user chat turns. **User `~/.claude/settings.json` hooks, `~/.claude.json`/`.mcp.json` MCP servers, and `CLAUDE.md` all load and fire normally.**
   - **Codex under T3 Code:** Runs via `codex app-server -c thread_unload_delay_secs=0` **without** `--ignore-user-config` or `features.hooks=false` on user threads. It explicitly preserves user `~/.codex/config.toml` MCP servers (even extending startup grace to 10s and injecting `T3CODE_THREAD_ID` into user stdio MCP servers) and symlinks `config.toml`, `hooks.json`, and `.hook-trust` in multi-instance shadow-home mode. **User Codex MCP servers and trusted hooks fire normally.**
   - **Antigravity under T3 Code (CRITICAL ISOLATION):** Runs `agy acp --gemini-dir <private-profile>` with an isolated `GEMINI_HOME` (`~/.t3/userdata/providers/antigravity/<sha256(instanceId)>`). T3 Code's source explicitly comments: `Private GEMINI_HOME isolates credentials/hooks/MCP; link global skills and rules from the real ~/.gemini.` Consequently, **global `~/.gemini/config/hooks.json` and `~/.gemini/config/mcp_config.json DO NOT LOAD when running Antigravity inside T3 Code.**
   - **Unified Passive Capture in T3 Code:** T3 Code has **no** user-facing MCP/hook settings of its own, but persists every thread across every provider into `~/.t3/userdata/statev2.sqlite` (SQLite WAL, 54 tables including `orchestration_v2_events` and `orchestration_v2_projection_messages`), making it an ideal optional passive ingestion source for a memory daemon.
5. **Memory LLM Backend Recommendation:** Do **not** rely on consumer OAuth subscriptions via `claude -p` or `codex exec` as the primary memory extraction engine. Direct API keys (e.g., Gemini Flash / OpenAI mini / Claude Haiku / OpenRouter) or local Ollama avoid ToS enforcement risk, rolling 5-hour subscription quota burn, cold-start latency (~1.5–4s per CLI spawn), and infinite hook-recursion hazards. If CLI reuse is offered as an opt-in power-user backend, always pass isolation flags (`claude -p --bare --no-session-persistence`, `codex exec --ephemeral -c features.hooks=false`, or `agy -p --disable-slash-commands`).

---

## 4-Client Integration Capability Matrix

| Capability | Claude Code (`2.1.331`) | Codex CLI (`0.172.0`) & Codex App | Antigravity (`Antigravity.app` / `agy`) | T3 Code (Beta `0.1.11`) |
| :--- | :--- | :--- | :--- | :--- |
| **MCP Config Paths** | **User/Local:** `~/.claude.json` (`mcpServers` & `projects.<path>.mcpServers`)<br>**Project:** `.mcp.json`<br>**Managed:** `/Library/Application Support/ClaudeCode/managed-mcp.json`<br>*(Note: `~/.claude/settings.json` only holds `.mcp.json` allow/deny policy, not server defs)* | **Global:** `~/.codex/config.toml` (`[mcp_servers.<name>]`)<br>**Project (trusted):** `.codex/config.toml`<br>**Shared across CLI, IDE ext, & Codex App** | **Global:** `~/.gemini/config/mcp_config.json` (`~/.gemini/antigravity/mcp_config.json` is symlinked here)<br>**Plugin:** `plugins/<name>/mcp_config.json` | **No user MCP config.** Runs internal HTTP MCP server (`t3-code`, `t3-code-direct`) for T3 orchestration.<br>**Pass-through:** Claude & Codex inherit user MCPs; **Antigravity global MCPs are isolated**. |
| **MCP Transports** | `stdio`, `http` (`streamable-http` alias), legacy `sse` (`type` required for URL entries) | `stdio` (`command`/`args`/`env`/`cwd`), Streamable HTTP (`url`, `bearer_token_env_var`, `http_headers`) | `stdio`, Streamable HTTP (`url` / legacy `serverUrl`). **Legacy HTTP+SSE unsupported.** | Internal server uses Streamable HTTP with per-session Bearer auth; ACP bridge (`AcpMcpStdioBridge`) for stdio-only ACP agents. |
| **Hooks Config Path** | `~/.claude/settings.json`, `.claude/settings.json`, `.claude/settings.local.json`, plugin `hooks/hooks.json` | `~/.codex/hooks.json`, `.codex/hooks.json` (trusted projects), plugin hooks (`[features] hooks = true`, `plugin_hooks = true`), legacy `notify = [...]` in `config.toml` | `~/.gemini/config/hooks.json`, `.agents/hooks.json` (workspace), `plugins/<name>/hooks.json` (plugin root) | **No user hook system.**<br>**Pass-through:** Claude & Codex user hooks fire; **Antigravity global hooks are isolated**. |
| **Hook Events (Memory-Relevant)** | **33 events** incl. `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PreCompact`, `PostCompact`, `Stop`, `SessionEnd`, `SubagentStart`, `SubagentStop` | **14 events** compiled in `0.172.0`: `SessionStart`, `UserPromptSubmit`, `PreModelRequest`, `PostModelRequest`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SubagentStart`, `SubagentStop`, `Stop`, `SessionEnd`, `Interrupt` | **6 events:** `SessionStart`, `PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation`, `Stop` | Inherits underlying driver hooks for Claude & Codex (except internal text-gen helpers & probes, where T3 disables hooks). |
| **Hook Trust / Gating** | Implicit trust for user `~/.claude/settings.json`; workspace trust dialog for project hooks | **Strict SHA-256 trust gate:** `[hooks.state]` in `config.toml` + `~/.codex/.hook-trust/` hashes; new/edited hooks skipped until approved (`/hooks`) unless `--dangerously-bypass-hook-trust` | Implicit for `~/.gemini/config/` & enabled plugins (`config.json`); `enabled: false` toggle per named hook | Inherits driver trust model (Codex still enforces `~/.codex` trust state). |
| **Hook Context Injection** | **Yes** (`SessionStart`, `UserPromptSubmit`, `PostToolUse`, `SubagentStart`): JSON `{"hookSpecificOutput":{"hookEventName":"...","additionalContext":"..."}}` (or raw stdout on `SessionStart`/`UserPromptSubmit`); >10KB auto-offloaded to file | **Yes** (`SessionStart`, `UserPromptSubmit`, `SubagentStart`, etc.): JSON `{"hookSpecificOutput":{"hookEventName":"...","additionalContext":"..."}}` *(avoid `systemMessage`—renders as yellow warning)* | **Yes** (`SessionStart`, `PreInvocation`, `PostInvocation`): JSON `{"injectSteps":[{"systemMessage":{"systemMessage":"..."}}]}` (supports `metadata.user_markdown` for UI-only display) | Native to Claude & Codex drivers; T3 also injects own orchestration instructions via SDK `systemPrompt.append` (Claude) and `thread/inject_items` (Codex). |
| **Hook Handler Types & Blocking** | `command`, `http`, `prompt`, `agent` (async supported on newer events); exit `2` blocks | `command`, `mcp_tool`, `prompt`, `agent` (`async` field present in schema); concurrent per event; exit `2` blocks | `command` only (`sh -c`); **synchronous & blocking** (30s default timeout) | N/A (delegated to drivers). |
| **Instructions / Rules Files** | `~/.claude/CLAUDE.md`, `./CLAUDE.md` (hierarchical), `.claude/rules/*.md`, skills (`~/.claude/skills/`) | `~/.codex/AGENTS.md`, `./AGENTS.md` (hierarchical), skills (`~/.codex/skills/`, `~/.agents/skills/`) | `~/.gemini/GEMINI.md`, `./GEMINI.md`, `./AGENTS.md`, `.agents/rules/*.md` (24KB/file, 20K-token aggregate budget), skills (`~/.gemini/config/skills/`, `.agents/skills/`) | No T3-native rules file; inherits driver rules. **Note:** Even for isolated Antigravity, T3 explicitly symlinks global `AGENTS.md`, `GEMINI.md`, and `skills/`. |
| **Session / Transcript Storage** | `~/.claude/projects/<encoded-cwd>/<session-uuid>.jsonl` (append-only JSONL, rich typed rows) | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` + `state_5.sqlite` (`threads`), `thread_history_1.sqlite`, `.codex-global-state.sqlite` | `~/.gemini/antigravity/brain/<conv-id>/.system_generated/logs/transcript.jsonl` (+ `transcript_full.jsonl`) + `conversation_summaries.db` (FTS5) | `~/.t3/userdata/statev2.sqlite` (SQLite WAL, 54 tables: `orchestration_v2_events`, `orchestration_v2_projection_messages`, `orchestration_v2_projection_threads`, FTS5 `orchestration_v2_message_search`) + native provider transcripts. |
| **Built-in Native Memory** | Auto-memory files in `~/.claude/projects/<cwd>/memory/` | Native `[features] memories = true` (`~/.codex/memories/`, `memories_1.sqlite`, `memories_v2_1.sqlite` with `stage1_outputs` + consolidation jobs) | `~/.gemini/antigravity/knowledge/` (knowledge items) | None (relies on providers + `orchestration_v2_projection_context_handoffs`). |
| **Headless / Non-Interactive CLI** | `claude -p` (`--output-format json\|stream-json`, `--json-schema`, `--bare`, `--no-session-persistence`) | `codex exec` (`--json`, `--output-schema`, `-o`, `--ephemeral`, `-c features.hooks=false`) | `agy -p` (`--output-format json\|stream-json`, `--json-schema`, `--disable-slash-commands`), plus `agy acp` | Electron desktop app + local Node/Effect server (`apps/server/dist/bin.mjs`), not a headless LLM CLI. |

---

## 1. Claude Code (`2.1.331`)

### 1.1 Hook Events & Lifecycle Cadences

Inspecting the shipped Claude Agent SDK (`apps/server/dist/sdk-DZX4LfOe.mjs` inside T3 Code's ASAR, matching Claude Code `2.1.331`) alongside the official documentation (`code.claude.com/docs/en/hooks`) reveals **33 supported hook events**:

```text
Session lifecycle : SessionStart, SessionEnd, Setup
Turn lifecycle    : UserPromptSubmit, UserPromptExpansion, Stop, StopFailure, MessageDisplay
Tool loop         : PreToolUse, PostToolUse, PostToolUseFailure, PostToolBatch,
                    PermissionRequest, PermissionDenied
Compaction        : PreCompact, PostCompact
Subagents / Tasks : SubagentStart, SubagentStop, TeammateIdle, TaskCreated, TaskCompleted
Model / Env / UI  : PreModelSwitch, PostModelSwitch, Notification, Elicitation, ElicitationResult,
                    ConfigChange, WorktreeCreate, WorktreeRemove, InstructionsLoaded,
                    CwdChanged, FileChanged, DirectoryAdded
```

**Memory-critical hook events and matchers:**

| Event | Fires When | Matcher Target | Primary Role for `0x1-ai-memory` |
| :--- | :--- | :--- | :--- |
| `SessionStart` | Session starts, resumes, clears, compacts, or forks | Source (`startup\|resume\|clear\|compact\|fork`) | Inject repo/user working memory snapshot via `additionalContext` |
| `UserPromptSubmit` | After user submits prompt, before model processes it | N/A | Semantic memory lookup for current prompt → inject targeted memories via `additionalContext` |
| `PostToolUse` | Immediately after a tool executes | Tool name regex (e.g., `Edit\|Write\|Bash`, `mcp__.*`) | Low-cost capture of code edits, commands, test outcomes (or queue transcript offset) |
| `PreCompact` | Immediately before context window compaction | Trigger (`manual\|auto`) | Trigger immediate memory extraction/flush before old turns are summarized away |
| `Stop` | After each complete assistant turn | N/A | Trigger incremental asynchronous extraction from `transcript_path` |
| `SessionEnd` | Session terminates (`clear`, `resume`, `logout`, exit) | Reason | Final session consolidation / summary job |

### 1.2 Hook Stdin / Stdout JSON Protocol

- **Config location:** `~/.claude/settings.json` (user), `.claude/settings.json` (project VCS), `.claude/settings.local.json` (project local), or plugin `hooks/hooks.json`. Verified locally in `~/.claude/settings.json` (`hooks` object keyed by event name → array of `{ matcher?, hooks: [{ type, command, timeout?, statusMessage? }] }`).
- **Input (`stdin`):** JSON object containing common fields (`session_id`, `transcript_path`, `cwd`, `hook_event_name`, `permission_mode`) plus event-specific fields (`prompt` for `UserPromptSubmit`; `tool_name`, `tool_use_id`, `tool_input` for `PreToolUse`; plus `tool_response`/`tool_output` for `PostToolUse`; `source` for `SessionStart`).
- **Exit codes:**
  - `0`: Success; `stdout` parsed as JSON (or plain text injected directly on `SessionStart`/`UserPromptSubmit`).
  - `2`: Hard block/reject (e.g., blocks tool in `PreToolUse` or compaction in `PreCompact`); `stderr` fed back to Claude.
- **Structured context injection (`stdout`):**
  ```json
  {
    "hookSpecificOutput": {
      "hookEventName": "UserPromptSubmit",
      "additionalContext": "<retrieved memories formatted concisely>"
    },
    "systemMessage": "Optional short terminal status line shown to user"
  }
  ```
  - **Critical schema rule:** `additionalContext` **must** be nested under `hookSpecificOutput` (top-level `additionalContext` is silently ignored).
  - **Truncation safeguard:** `additionalContext` strings exceeding ~10,000 characters are automatically spilled to a file with a truncated preview snippet—keep injected memory payloads well under ~4–6 KB.

### 1.3 MCP Configuration Paths, Scopes & Transports

Verified via `~/.local/bin/claude mcp add --help`, `~/.claude.json` inspection, and `code.claude.com` docs:

| Scope (`claude mcp add -s`) | Storage File | Key Path | Approval Required? |
| :--- | :--- | :--- | :--- |
| `user` | `~/.claude.json` | Top-level `mcpServers` | No |
| `local` *(default)* | `~/.claude.json` | `projects["<abs-cwd>"].mcpServers` | No |
| `project` | `<repo>/.mcp.json` | Top-level `mcpServers` | Yes (on first use; tracked in `~/.claude.json` `enabledMcpjsonServers` / `~/.claude/settings.json` `enableAllProjectMcpServers`) |
| Managed (enterprise) | `/Library/Application Support/ClaudeCode/managed-mcp.json` (macOS) | Top-level `mcpServers` | Policy-enforced (excludes other MCP configs & forbids `--strict-mcp-config`) |

- **Important distinction:** `~/.claude/settings.json` does **not** define `mcpServers`; it only holds policy/approval keys (`enableAllProjectMcpServers`, `enabledMcpjsonServers`, `disabledMcpjsonServers`, `allowedMcpServers`, `deniedMcpServers`, `disableAllHooks`). Note also `~/.claude/.claude.json` exists locally as a minimal metadata/migration state file, while actual `mcpServers` live in **`~/.claude.json`**.
- **Transports:**
  - `stdio`: `{ "type": "stdio", "command": "...", "args": [...], "env": {...} }`
  - `http` (recommended for local daemon; `streamable-http` accepted as alias): `{ "type": "http", "url": "http://127.0.0.1:<port>/mcp", "headers": {...} }`. **Note:** `type` is required on URL entries; omitting it defaults to `stdio` and fails.
  - `sse`: Legacy fallback (`type: "sse"`).
  - Environment variable expansion `${VAR}` is supported in `.mcp.json`.

### 1.4 Session JSONL Storage Format

Verified on local `~/.claude/projects/<encoded-cwd>/<session-uuid>.jsonl` (330 files observed locally; `<encoded-cwd>` replaces `/` with `-`):

- Append-only JSONL where each line has a `type` discriminator. Observed `type` values and top-level keys:
  - `user`: `uuid`, `parentUuid`, `sessionId`, `timestamp`, `cwd`, `gitBranch`, `isSidechain`, `promptId`, `entrypoint`, `message`, `version`, `turnOrigin`, `turnPosition`
  - `assistant`: `uuid`, `parentUuid`, `sessionId`, `timestamp`, `cwd`, `gitBranch`, `isSidechain`, `message`, `requestedModel`, `requestId`, `thinkingDurationMs`, `entrypoint`, `version`
  - `system`: `uuid`, `parentUuid`, `sessionId`, `subtype`, `hookCount`, `hookInfos`, `hookAdditionalContext`, `hookErrors`, `stopReason`, `preventedContinuation`
  - `attachment`, `custom-title`, `last-prompt`, `tools-used`, `cost-state`, `agent-name`, `queue-operation`, `session-owner`
- **Daemon note:** Filter out `isSidechain: true` if you only want primary conversation turns (or tag sidechains as subagent activity), and deduplicate by `uuid`.

---

## 2. Codex CLI (`0.172.0`) & Codex App

### 2.1 MCP Configuration (`~/.codex/config.toml`)

Verified directly from `~/.codex/config.toml` and `codex mcp add --help`:

- **Global path:** `~/.codex/config.toml` (or `$CODEX_HOME/config.toml`)
- **Project path:** `.codex/config.toml` (loaded only when the project path is trusted in `[projects."<path>"]`)
- **Transports (`codex mcp add <name> (--url <URL> | -- <COMMAND>...)`):**
  ```toml
  # Stdio transport (local process)
  [mcp_servers.0x1-memory]
  command = "/abs/path/to/0x1-memory-mcp"
  args = ["serve"]
  startup_timeout_sec = 30.0
  tool_timeout_sec = 60.0
  enabled = true

  [mcp_servers.0x1-memory.env]
  MEMORY_DB_PATH = "/Users/<user>/.0x1-memory/memory.db"

  # Granular per-tool approval override (observed in local config.toml!)
  [mcp_servers.0x1-memory.tools.memory_search]
  approval_mode = "approve"

  # Streamable HTTP transport (remote or local HTTP daemon)
  [mcp_servers.0x1-memory-http]
  url = "http://127.0.0.1:37777/mcp"
  bearer_token_env_var = "MEMORY_DAEMON_TOKEN"
  enabled_tools = ["memory_search", "memory_save"]
  disabled_tools = ["memory_purge"]
  default_tools_approval_mode = "auto"
  ```
- **Per-tool approval insight:** Local `~/.codex/config.toml` confirms Codex supports `[mcp_servers.<name>.tools.<tool_name>] approval_mode = "approve"` (e.g., used by `context-mode` and `rea`), allowing read-only memory tools to execute frictionlessly without user prompts.

### 2.2 Hooks & Legacy `notify` System

Inspecting `~/.codex/hooks.json`, `~/.codex/config.toml`, `~/.codex/.hook-trust/`, installed plugin hooks (`ponytail`), and static strings from the `codex-cli 0.172.0` Mach-O binary (`304 MB`) establishes the exact hook architecture:

- **Feature flags in `~/.codex/config.toml`:**
  ```toml
  [features]
  hooks = true
  plugin_hooks = true
  memories = true      # Codex also ships its own native memory pipeline
  multi_agent = true
  ```
- **Config files:**
  - Global: `~/.codex/hooks.json`
  - Project (trusted): `.codex/hooks.json`
  - Plugins: plugin-bundled `hooks/*.json` (when `plugin_hooks = true`)
  - Legacy `notify = ["script", ...]` in `config.toml`: still supported for simple turn-end notifications, but superseded by `hooks.json` for full lifecycle control.
- **Complete 14-event set compiled into `codex-cli 0.172.0`:**
  `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `PreModelRequest`, `PostModelRequest`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SubagentStart`, `SubagentStop`, `Stop`, `Interrupt`.
  *(Locally active in `~/.codex/hooks.json` via `context-mode`/`impeccable`/`graphify`: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PreCompact`, `Stop`.)*
- **Handler types compiled into `0.172.0`:** `command` (with optional `commandWindows`, `timeoutSec`, `async`, `additionalContextLimit`, `statusMessage`), `mcp_tool`, `prompt`, `agent`.
- **Stdin / Stdout JSON protocol (Claude-compatible):**
  - `stdin`: `{ session_id, transcript_path, cwd, hook_event_name, model, turn_id, ... }` + event fields (`prompt`, `tool_name`, `tool_use_id`, `tool_input`, `tool_response`, `stop_hook_active`, `last_assistant_message`).
  - `stdout` (exit `0`):
    ```json
    {
      "hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": "<retrieved memories>"
      }
    }
    ```
  - **Observed UI Gotcha (verified in installed `ponytail-runtime.js`):** Do **not** include top-level `systemMessage` in Codex hook JSON output—Codex maps `systemMessage` to a yellow `warning:` badge in the TUI, making normal memory injection look like an error. Emit only `hookSpecificOutput.additionalContext`.
  - Exit `2` blocks the action (feedback via `stderr`). Multiple matching command hooks execute concurrently.
  - Events that do not support `additionalContext` log `ignoring additionalContextLimit ... this event cannot emit additionalContext` (observed in binary strings).

- **CRITICAL — Codex Hook Trust Model (`[hooks.state]` & `~/.codex/.hook-trust/`):**
  Unlike Claude Code and Antigravity, **Codex will NOT execute newly written or modified unmanaged hooks in `~/.codex/hooks.json` until explicitly trusted!**
  - Observed locally in `~/.codex/config.toml`:
    ```toml
    [hooks.state."/Users/<user>/.codex/hooks.json:session_start:0:0"]
    trusted_hash = "sha256:<hex>"
    enabled = true
    ```
  - Observed locally in `~/.codex/.hook-trust/`: directory of `<sha256-key>` files, each containing `sha256:<hex>`.
  - **Installer implication:** When `0x1-ai-memory install codex` adds hooks to `~/.codex/hooks.json`, the user must approve them in Codex's interactive `/hooks` review UI (or run `codex exec --dangerously-bypass-hook-trust` in trusted automation). Never mutate `~/.codex/.hook-trust` or bypass trust silently—prompt the user to run `/hooks` once after installation, or keep the hook entrypoint script a thin static shim whose command string never changes after initial approval!

### 2.3 Instructions (`AGENTS.md`), SQLite Databases & Session JSONL

- **Instructions:** `~/.codex/AGENTS.md` (global) + hierarchical `AGENTS.md` walking from repo root to CWD.
- **Session JSONL:** `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<id>.jsonl`.
  - Observed entry `type` values & `payload` keys:
    - `session_meta`: `session_id`, `id`, `cwd`, `git`, `cli_version`, `model_provider`, `originator`, `source`, `base_instructions`, `runtime_workspace_roots`, `timestamp`
    - `turn_context`: `turn_id`, `root_turn_id`, `cwd`, `model`, `effort`, `approval_policy`, `sandbox_policy`, `collaboration_mode`
    - `response_item`: `id`, `role`, `type`, `tools` (assistant/user messages & tool calls)
    - `event_msg`: `thread_id`, `thread_settings`, `type`
    - `token_usage_record`: `session_id`, `thread_id`, `turn_id`, `usage`, `thread_token_usage`, `turn_token_usage`
    - `world_state`: `full`, `state`
- **Local SQLite databases (table schemas observed read-only, no user content read):**
  - `~/.codex/.codex-global-state.sqlite`: 1 table — `settings(scope, key, value)`
  - `~/.codex/state_5.sqlite`: 18 tables — core ones: `threads(id, rollout_path, created_at, updated_at, source, ...)`, `projects`, `project_roots`, `thread_spawn_edges`, `thread_dynamic_tools`, `thread_attachments`, `session_model_ids`, `external_agent_config_imports`
  - `~/.codex/thread_history_1.sqlite`: `thread_history_projection_state(thread_id, next_rollout_byte_offset, next_rollout_ordinal)` + `thread_realtime_items`
  - `~/.codex/memories_1.sqlite` & `memories_v2_1.sqlite`: Codex's built-in memory pipeline (`stage1_outputs(thread_id, source_updated_at, raw_memory, rollout_summary, rollout_slug, ...)`, `jobs`, `consolidation_progress`). *Design note:* `0x1-ai-memory` should note Codex's native `[features] memories = true` to avoid redundant memory bloat or optionally offer import/interoperability.

### 2.4 Does Codex App Share `~/.codex` with Codex CLI?

- **Local observation:** `/Applications/Codex.app` is **not** currently installed on this machine, though `~/.codex/models_cache.codex_work_desktop.json` is present inside `~/.codex/`, confirming desktop-app state writes into the same `~/.codex/` directory.
- **Official OpenAI Codex docs:** The Codex macOS desktop app, Codex CLI, and IDE extensions **share the exact same `~/.codex/config.toml`, `~/.codex/hooks.json`, hook trust state, `AGENTS.md`, and session storage** (driven by `codex app-server`).
- **macOS Desktop App `PATH` Caveat:** macOS GUI apps launched from Finder/Dock do **not** inherit `~/.zshrc` `PATH` additions (e.g., Homebrew `/opt/homebrew/bin`, `fnm`, `~/.local/bin`). Always register MCP `command` and hook executables in `~/.codex/config.toml` and `~/.codex/hooks.json` with **absolute binary paths** (or connect over Streamable HTTP `url = "http://127.0.0.1:<port>/mcp"`).

---

## 3. Antigravity (`Antigravity.app` & `agy` CLI)

*Primary sources: `~/.gemini/antigravity/builtin/skills/agy-customizations/SKILL.md` and its companion `docs/{hooks,mcp_servers,rules,skills,plugins,json_configs}.md`, `~/.local/bin/agy --help`, and `~/.gemini/` filesystem inspection.*

### 3.1 MCP Server Registration

- **Global config:** `~/.gemini/config/mcp_config.json` (verified locally: `~/.gemini/antigravity/mcp_config.json` is a **symlink** pointing directly to `~/.gemini/config/mcp_config.json`).
- **Plugin config:** `~/.gemini/config/plugins/<plugin>/mcp_config.json` or `.agents/plugins/<plugin>/mcp_config.json`.
- **CLI management:** `agy mcp add [-t stdio|http] [-e K=V] [-H "K: V"] <name> <commandOrUrl> [args...]`, plus `remove`, `list`, `enable`, `disable`.
- **Schema & Transports:**
  ```json
  {
    "mcpServers": {
      "0x1-memory-stdio": {
        "command": "/abs/path/to/0x1-memory-mcp",
        "args": ["serve"],
        "env": { "MEMORY_DB": "/path/to/db" },
        "enabledTools": ["memory_search", "memory_save"]
      },
      "0x1-memory-http": {
        "url": "http://127.0.0.1:37777/mcp"
      }
    }
  }
  ```
  - Supports **Stdio** and **Streamable HTTP** (`url` or legacy `serverUrl`).
  - **Legacy HTTP+SSE (MCP 2024-11-05) is NOT supported.**

### 3.2 Lifecycle Hooks (`hooks.json`)

- **Locations:**
  - Global: `~/.gemini/config/hooks.json`
  - Workspace: `.agents/hooks.json` (also `.agent/`, `_agents/`, `_agent/`)
  - Plugin: `plugins/<name>/hooks.json` (**strictly at plugin root** for native plugins; `agy plugin install` stages `hooks/hooks.json` only when importing Claude Code or Gemini CLI extensions).
- **Schema:** Top-level object keyed by arbitrary **hook name** (not `"hooks": {...}`!), with optional `"enabled": false`:
  ```json
  {
    "0x1-memory": {
      "SessionStart": [
        { "type": "command", "command": "/abs/path/to/0x1-hook-agy", "timeout": 10 }
      ],
      "PreInvocation": [
        { "type": "command", "command": "/abs/path/to/0x1-hook-agy", "timeout": 10 }
      ],
      "PostToolUse": [
        {
          "matcher": "run_command|write_file|edit_file",
          "hooks": [
            { "type": "command", "command": "/abs/path/to/0x1-hook-agy", "timeout": 10 }
          ]
        }
      ],
      "Stop": [
        { "type": "command", "command": "/abs/path/to/0x1-hook-agy", "timeout": 15 }
      ]
    }
  }
  ```
  - **Notice the structural split:** `SessionStart`, `PreInvocation`, `PostInvocation`, and `Stop` take a **flat array of handler objects directly**, whereas `PreToolUse` and `PostToolUse` require the **grouped `{ matcher, hooks: [...] }` wrapper**!
  - **CWD rule:** Hook commands run via `sh -c` with process CWD set to the directory containing `hooks.json` (the agent's working directory is passed as `cwd` in stdin JSON). Plugin hooks receive `PLUGIN_ROOT` and `PLUGIN_DATA` env vars.

- **Supported Events (6 total):**
  1. `SessionStart` — once when a new session starts.
  2. `PreInvocation` — before every model call (`invocationNum`, `initialNumSteps`, `lastUserInput`). *Note: fires before EVERY model step in a turn, not only on user prompt submit—deduplicate/guard on `lastUserInput` or `invocationNum` so memories aren't re-injected redundantly on every tool-loop iteration!*
  3. `PreToolUse` — before tool execution; returns `{ decision: "allow"|"deny"|"ask"|"force_ask", reason?, permissionOverrides?, overwrite? }`.
  4. `PostToolUse` — after tool execution (`toolCall: { id, name, args }`, `stepIdx`, `error?`); expects `{}` on stdout.
  5. `PostInvocation` — after tool calls finish; can return `injectSteps` and `terminationBehavior: "force_continue"|"terminate"|""`.
  6. `Stop` — when execution loop terminates (`executionNum`, `terminationReason`, `fullyIdle`, `continuationActive`); returns `{ decision: "continue"|"", reason? }`.

- **Stdin / Stdout JSON Contract (camelCase protojson!):**
  - Common `stdin` fields: `conversationId`, `workspacePaths`, `transcriptPath`, `artifactDirectoryPath`, `modelName`, `lastUserInput`, `hookEventName`, `cwd`.
  - **Context Injection (`SessionStart`, `PreInvocation`, `PostInvocation`) stdout:**
    ```json
    {
      "injectSteps": [
        {
          "systemMessage": {
            "systemMessage": "<retrieved memories visible to the model>",
            "metadata": {
              "user_markdown": "*Loaded 4 relevant memories from 0x1-memory*"
            }
          }
        }
      ]
    }
    ```
    - Note the nested `systemMessage.systemMessage` structure!
    - Setting `metadata.user_markdown` renders a clean inline UI notice while the model only reads `systemMessage.systemMessage`.
    - Also supports `userMessage` (string) and `errorMessage: { userMessage: "..." }` (paired with `"terminationBehavior": "terminate"`).
- **Current Limitations (official `hooks.md`):**
  - Only `type: "command"` is supported (no HTTP or prompt hooks yet).
  - **Synchronous & blocking** (default 30s timeout): keep memory hook latency <150ms and offload heavy extraction to a background daemon on `Stop`!

### 3.3 Rules, Skills, Plugins & Transcripts

- **Loading Priority (highest → lowest):**
  1. Workspace `.agents/` (walks up from CWD to `.git` root) & hierarchical `GEMINI.md` / `AGENTS.md` / `.agents/rules/*.md`
  2. Workspace declared JSON configs (`.agents/skills.json`, `.agents/plugins.json`)
  3. Global discovery (`~/.gemini/config/`)
  4. Built-in customizations (`~/.gemini/antigravity/builtin/`)
  5. Global declared JSON configs
- **Rules Budget:** 24,000 bytes per rule file (truncated on line boundaries); 20,000-token aggregate rules budget (`defaultRulesBudget`) across always-on/global rules (over-budget rules demoted to file path pointers).
- **Plugins (Recommended Distribution Packaging for Antigravity!):** A single Antigravity plugin folder (`plugins/0x1-memory/`) can bundle `plugin.json`, `mcp_config.json`, `hooks.json` (at plugin root), `rules/AGENTS.md`, and `skills/<name>/SKILL.md` as one atomic unit, toggleable via `agy plugin enable|disable` (persisted in `~/.gemini/config/config.json`).
- **Transcript Storage (verified locally across 466 conversations):**
  - Compact transcript: `~/.gemini/antigravity/brain/<conversation-id>/.system_generated/logs/transcript.jsonl`
  - Full untruncated transcript: `~/.gemini/antigravity/brain/<conversation-id>/.system_generated/logs/transcript_full.jsonl`
  - Note product-specific directories: `antigravity/` (Antigravity 2.0), `antigravity-cli/` (CLI), `antigravity-ide/` (IDE).
  - Observed JSONL `type` values: `USER_INPUT`, `PLANNER_RESPONSE` (`model`, `input_tokens`, `output_tokens`, `cache_read_tokens`, `thinking`, `tool_calls`, `step_index`, `created_at`), `SYSTEM_MESSAGE`, `GENERIC`.
  - Summary & FTS5 index DB: `~/.gemini/antigravity/conversation_summaries.db` (`conversation_summaries`, `conversations_fts` FTS5, `conversation_parents`).

---

## 4. T3 Code (`0.1.11`) — Shipped-Artifact Reverse Engineering Findings

*Evidence source: Static inspection of `/Applications/T3 Code (Beta).app/Contents/Resources/app.asar` (`CFBundleShortVersionString = 0.1.11`, Electron app with bundled Effect/Node server in `apps/server/dist/{server-8K5Ixyrs,runtimeLayer-CyWe8jGq,sdk-DZX4LfOe}.mjs`) and read-only schema inspection of `~/.t3/userdata/`. (Note: `npx -y rea-agents@6.8.0 analyze-javascript-application` flagged an unpacked native binary signature mismatch on `@clerk/electron-passkeys-darwin-arm64/electron-passkeys.darwin-arm64.node` and exceeded the 3-minute MCP call timeout on the ~18 MB combined server JS bundles, so targeted read-only ASAR module extraction was performed directly to retrieve verbatim TypeScript-annotated source modules.)*

### 4.1 How T3 Code Runs Claude Code (`ClaudeAdapterV2.ts`, `ClaudeHome.ts`, `sdk-DZX4LfOe.mjs`)

- **Mechanism:** T3 Code uses the bundled **`@anthropic-ai/claude-agent-sdk`** (`apps/server/dist/sdk-DZX4LfOe.mjs`) calling `query()`, which spawns the local `claude` executable (`pathToClaudeCodeExecutable`, falling back to user's `claude` binary).
- **Exact Environment & Config Directory (`src/provider/Drivers/ClaudeHome.ts`):**
  - Resolves `CLAUDE_CONFIG_DIR` in priority order: explicit T3 instance `config.homePath` → inherited `CLAUDE_CONFIG_DIR` → default **`~/.claude`**.
  - Sets `CLAUDE_CODE_ENTRYPOINT="t3-code"` (unless overridden by user's `settings.json` `env`), sets `T3CODE_THREAD_ID=<threadId>`, and strips host-nesting vars (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `ELECTRON_RUN_AS_NODE`, etc.).
- **Exact Isolation Flag Audit (`makeClaudeQueryOptions` in `apps/server/dist/runtimeLayer-CyWe8jGq.mjs` & arg builder in `sdk-DZX4LfOe.mjs`):**
  - `settingSources`: **OMITTED (`undefined`)** on user chat sessions. Verified in `sdk-DZX4LfOe.mjs` (`@1397946`):
    ```js
    if (T && Object.keys(T).length > 0) B.push(...iS("--mcp-config", ce({ mcpServers: T })));
    if (x !== void 0) B.push(Ye("--setting-sources", x.join(",")));
    if (w) B.push("--strict-mcp-config");
    ```
    Because `x` (`settingSources`) is `undefined`, **`--setting-sources` is NOT passed** to the spawned `claude` subprocess—Claude Code uses its full default setting sources (`user`, `project`, `local`).
  - `strictMcpConfig`: **NOT set (`undefined`)** on user chat sessions.
  - `disableAllHooks`: **NOT set** on user chat sessions. *(T3 Code DOES pass `settings: { disableAllHooks: true }`, `strictMcpConfig: true`, `mcpServers: {}`, and `ENABLE_CLAUDEAI_MCP_SERVERS: "false"` ONLY during its capabilities health probe in `src/provider/ClaudeProvider.ts` and one-shot title/text-gen helper `src/textGeneration/ClaudeTextGeneration.ts`—never on user chat turns.)*
  - `mcpServers` injection (`claudeMcpQueryOverrides`): T3 Code strips `mcpServers` before spawning (`const { mcpServers, ...sdkOptions } = input.options`) and dynamically registers its own HTTP server `{"t3-code": { type: "http", url, headers: { Authorization } }}` via `queryRuntime.setMcpServers(mcpServers)` (`mcp_set_servers` control request). This adds `t3-code` alongside—not replacing—user `~/.claude.json` and project `.mcp.json` servers.
  - `hooks`: T3 Code registers its own SDK `Stop` callback (for wake/goal handling) and optional `SubagentStart` callback (for device tool instructions), which run alongside user hooks.
  - `systemPrompt`: Uses `{ type: "preset", preset: "claude_code", append: ... }` (appending T3 orchestration instructions while preserving Claude Code's native system prompt & `CLAUDE.md` loading).
- **VERDICT (Claude Code in T3 Code):** **FULL PASS-THROUGH.** User `~/.claude/settings.json` hooks, `~/.claude.json` & `.mcp.json` MCP servers, `CLAUDE.md`, and `~/.claude/projects/` session JSONLs all work normally (unless the user explicitly sets a custom `homePath` or `--strict-mcp-config` / `--setting-sources` in T3 Code's provider `launchArgs`).

### 4.2 How T3 Code Runs Codex (`CodexAdapterV2.ts`, `CodexHomeLayout.ts`, `CodexManagedRuntime.ts`)

- **Mechanism:** T3 Code spawns **`codex app-server`** as a long-lived child process per provider instance (JSON-RPC over stdio) via `codexAppServerArgs`:
  ```js
  const codexAppServerArgs = (launchArgs) => [
    "app-server",
    "-c",
    "thread_unload_delay_secs=0",
    ...codexLaunchArgv(launchArgs)
  ];
  ```
- **Exact `CODEX_HOME` & Shadow-Home Behavior (`src/provider/Drivers/CodexHomeLayout.ts` & `CodexManagedHome.ts`):**
  - **Default `"codex"` instance (`mode: "direct"`):** Uses `~/.codex` directly (`CODEX_HOME` is unset unless user configured `config.homePath`).
  - **Secondary / Multi-Account Codex instances (`mode: "authOverlay"` with `shadowHomePath` under `~/.t3/userdata/providers/codex/<instanceId>/shadow`):** T3 Code sets `CODEX_HOME` to the shadow directory and **symlinks every entry from `~/.codex` into the shadow directory** EXCEPT:
    - `PRIVATE_ENTRY_NAMES`: `auth.json`, `auth.lock`, `models_cache.json` (kept private per account)
    - `SHADOW_LOCAL_ENTRY_NAMES`: `log`, `memories`, `tmp`, `.DS_Store` (kept shadow-local)
    - **Crucially:** `config.toml`, `hooks.json`, `.hook-trust`, `AGENTS.md`, `sessions/`, `archived_sessions/`, `skills/`, `plugins/`, and `sqlite/` **ARE ALL SYMLINKED** from `~/.codex`! *(Note: Codex's native `memories/` directory is shadow-local in multi-instance mode, whereas external `0x1-ai-memory` via MCP/hooks stays completely shared across instances!)*
- **Exact MCP & Hook Preservation (`codexThreadRuntimeParams` & `CODEX_THREAD_CONFIG` in `server-8K5Ixyrs.mjs`):**
  - On `thread/start` and `thread/resume`, T3 Code passes:
    - `CODEX_THREAD_CONFIG`: `{ "tools.update_plan.enabled": true, mcp_optional_startup_grace_ms: 10000, "features.concurrent_reasoning_summaries": true }` — notably raising MCP startup grace from 1s to **10s** explicitly so cold user `npx`/`uvx` MCP servers start cleanly before the first turn!
    - Reads the user's configured stdio MCP servers via `config/read` (`codexStdioMcpServerNames`) and injects `mcp_servers.<name>.env.T3CODE_THREAD_ID = <threadId>` into **every user stdio MCP server**!
    - Adds T3's own HTTP servers (`mcp_servers.t3-code` and `mcp_servers.t3-code-direct`) without removing user `[mcp_servers.*]`.
  - **Hooks:** Does **NOT** pass `--ignore-user-config` or `features.hooks=false` on user threads. *(Those flags ARE passed ONLY in `src/textGeneration/CodexTextGeneration.ts` for T3's ephemeral background title/branch-name helper calls.)*
  - Additional T3 instructions are injected via `thread/inject_items` (`role: "developer"`), leaving `AGENTS.md` intact.
- **VERDICT (Codex in T3 Code):** **FULL PASS-THROUGH.** User `~/.codex/config.toml` MCP servers, trusted `~/.codex/hooks.json` hooks, `AGENTS.md`, and `~/.codex/sessions/` all work normally! *(Caveat: Codex's hook trust gate still applies—hooks must be trusted in `~/.codex/config.toml` `[hooks.state]`.)*

### 4.3 How T3 Code Runs Antigravity (`AntigravityDriver.ts`, `antigravityAuthSupport.ts`, `AntigravityAdapterV2.ts`)

- **Mechanism:** T3 Code spawns **`agy acp`** via `buildAntigravityAcpSpawnInput`:
  ```js
  args: ["acp", "--gemini-dir", input.profile.geminiHome, "--app-data-dir", "antigravity-acp"]
  ```
  with `env.GEMINI_HOME = ~/.t3/userdata/providers/antigravity/<sha256(instanceId)>`.
- **CRITICAL ISOLATION FINDING (`linkAntigravityUserConfig` in `server-8K5Ixyrs.mjs`):**
  Verbatim source comment from T3 Code:
  ```js
  /** Private GEMINI_HOME isolates credentials/hooks/MCP; link global skills and rules from the real ~/.gemini. */
  ```
  `linkAntigravityUserConfig` symlinks **ONLY**:
  1. User global skills directories (`skills/`)
  2. Global rule files (`AGENTS.md`, `GEMINI.md`)
  It intentionally does **NOT** symlink `~/.gemini/config/hooks.json`, `~/.gemini/config/mcp_config.json`, or `~/.gemini/config/plugins/`.
- **VERDICT (Antigravity in T3 Code):** **GLOBAL HOOKS & GLOBAL MCP ARE ISOLATED.**
  - Global `~/.gemini/config/hooks.json` and `~/.gemini/config/mcp_config.json` **will NOT load** when running Antigravity inside T3 Code.
  - Project-scoped `.agents/hooks.json` inside a repository CWD is discovered relative to workspace CWD (not `GEMINI_HOME`), and global `AGENTS.md`/`GEMINI.md` + `skills/` ARE symlinked.
  - To capture Antigravity sessions run inside T3 Code without modifying T3's private provider dirs, watch `~/.t3/userdata/statev2.sqlite` (below) and/or place project `.agents/` customizations where appropriate.

### 4.4 T3 Code's Own Storage (`~/.t3/userdata/statev2.sqlite`) & MCP/Hook Settings

- **Own MCP / Hook Config:** Verified across `Schema-BBjlTC3i.mjs`, `server-8K5Ixyrs.mjs`, and `~/.t3/userdata/{settings,client-settings,desktop-settings}.json`: T3 Code has **no** user-configurable MCP server list or user hook system of its own. Its `src/mcp/McpHttpServer.ts` exists solely to serve T3's internal orchestration tools (`t3-code`) to spawned agents, and its `mcp_app_model_context` table caches UI context from MCP App tool responses.
- **Unified Watchable Thread Store (`~/.t3/userdata/statev2.sqlite`):**
  - SQLite in **WAL mode** (`statev2.sqlite`, `-wal`, `-shm`), **54 tables**, event-sourced via `orchestration_v2_*`:
    - `orchestration_v2_events(sequence, event_id, thread_id, run_id, node_id, provider, driver, event_type, occurred_at, payload_json)` — monotonic `sequence` primary key makes incremental cursor polling trivial (`WHERE sequence > ?`)!
    - `orchestration_v2_projection_threads(thread_id, project_id, title, default_provider, runtime_mode, active_provider_thread_id, updated_at, archived_at, deleted_at, payload_json)`
    - `orchestration_v2_projection_messages(message_id, thread_id, run_id, node_id, role, streaming, created_at, updated_at, payload_json, search_text)` — includes normalized `search_text` and `streaming = 0` completion flag!
    - `orchestration_v2_projection_turn_items(turn_item_id, thread_id, run_id, provider_thread_id, provider_turn_id, ordinal, type, status, payload_json)`
    - `orchestration_v2_projection_provider_threads(provider_thread_id, thread_id, provider, driver, provider_session_id, status, ...)` — maps T3 `thread_id` directly to native Claude/Codex/Antigravity session IDs for **cross-source deduplication**!
    - `projection_projects(project_id, title, workspace_root, ...)` — maps `project_id` to canonical repo path.
  - **Safe Read-Only Watching Pattern:** Open read-only (`sqlite3` with `mode=ro` + `busy_timeout=2000`; never write to T3's DB!), trigger on mtime/WAL changes or short poll interval, query `orchestration_v2_projection_messages WHERE streaming = 0 AND updated_at > ?` joined with `orchestration_v2_projection_provider_threads` so threads already ingested via Claude/Codex hooks are cleanly deduplicated via `provider_thread_id`!

### 4.5 Status in 0x1-memory: T3 watcher removed (Spec #1)

The read-only `statev2.sqlite` watcher described above was removed, along with the `t3-code` Client, its Connections card and the `t3_watcher_enabled` setting. It never captured anything: it read an empty `search_text` column and deduplicated on the wrong id. T3 Code is a **Host**, not a **Client**: it launches Claude Code and Codex, whose own hooks and MCP entries already reach the Daemon. The Daemon only records the Host for a session; it never reads a Host's private storage.

On open, the Daemon deletes the leftover `t3-code` connection row and the `t3_watcher_enabled` / `_meta:t3_watermark` settings.

**Not captured, by design:**

- **Antigravity inside T3 Code.** T3 starts `agy acp` with a private `GEMINI_HOME` (4.3), so the global 0x1-memory hooks and MCP entry never load. Run Antigravity outside T3 (the app, `agy` in a terminal, or an Orca terminal) when its memory matters.
- **Orca chat mode for Antigravity with a non-Google sign-in.** It has the same isolation. Orca terminals, and Claude Code or Codex chats on the default account, use the user config and are captured normally.

---

## 5. Subscription / Headless CLI Reuse Feasibility (Memory LLM Backend)

When `0x1-ai-memory` runs background LLM calls (fact extraction, entity/relation updates, session summarization, conflict resolution), should it reuse existing subscriptions by shelling out to `claude -p`, `codex exec`, or `agy -p`, or call model APIs directly?

### 5.1 Observed Non-Interactive CLI Capabilities

| CLI | Non-Interactive Command | Structured Output | Isolation Flags (MANDATORY to prevent recursion!) |
| :--- | :--- | :--- | :--- |
| **Claude Code** | `claude -p "<prompt>"` | `--output-format json` + `--json-schema '<schema>'` | `--bare --no-session-persistence --tools "" --strict-mcp-config` *(NOTE: `--help` states `--bare` skips hooks/OAuth and requires API key or `apiKeyHelper`! Under OAuth login, omit `--bare` and pass `--settings '{"disableAllHooks":true}' --strict-mcp-config --tools "" --no-session-persistence`)* |
| **Codex CLI** | `codex exec "<prompt>"` | `--output-schema <file>` + `-o <out-file>` (or `--json`) | `--ephemeral --skip-git-repo-check -s read-only -c features.hooks=false -c features.codex_hooks=false` *(and disable MCP servers as T3's `CodexTextGeneration.ts` does!)* |
| **Antigravity** | `agy -p "<prompt>"` | `--output-format json` + `--json-schema '<schema>'` | `--disable-slash-commands` *(Note: no built-in `--no-hooks` flag observed in `agy --help`; hooks must guard against recursion via an env var like `OX1_MEMORY_INTERNAL=1`!)* |

### 5.2 Critical Trade-offs: Subscription CLI Reuse vs Direct API / Local LLM

| Dimension | Headless CLI Reuse (`claude -p` / `codex exec` / `agy -p`) | Direct API Key (Gemini / OpenAI / Anthropic / OpenRouter) | Local LLM (Ollama / llama.cpp / MLX) |
| :--- | :--- | :--- | :--- |
| **ToS & Account Enforcement Risk** | **HIGH (Anthropic), MODERATE (OpenAI).** Anthropic explicitly restricts consumer/Pro/Max OAuth tokens to ordinary use inside official Claude Code/Claude.ai and performs server-side enforcement against third-party harnesses; automated SDK/`claude -p` workflows require API keys or eligible linked Console credit pools. OpenAI consumer Plus/Pro plans are subject to consumer data-training policies unless opted out. | **LOW / COMPLIANT.** Platform API ToS explicitly permits automated background pipelines and excludes API traffic from model training by default. | **ZERO.** 100% local, private, zero external ToS risk. |
| **Quota & Rate Limits** | **Severe interference:** Background extraction eats directly into the developer's interactive 5-hour rolling window (Claude Pro/Max, ChatGPT Plus) or weekly ceiling, risking lockouts during active coding! | Independent pay-as-you-go TPM/RPM limits. Extracting memories with small fast models (e.g., Gemini 2.5 Flash-Lite / GPT-5-nano / Haiku) costs fractions of a cent per session (typically \$0.50–\$3/month). | Hardware-bound (free, unlimited on Apple Silicon). |
| **Latency & Overhead** | **High (~1.5s–6s cold start per call)** from Node/Rust/Go CLI bootstrap, auth checks, system prompt overhead, and tool/config loading. Unusable on synchronous hooks (`UserPromptSubmit`); sluggish even in background queues. | **Low (~250ms–800ms)** via direct HTTP/keep-alive JSON mode with tiny extraction prompts. | **Low–Medium (~200ms–1.5s)** on Apple Silicon M-series (MLX/Ollama 4B–9B models). |
| **Hook Recursion Hazard** | **Critical footgun:** If `Stop` hook invokes `claude -p` or `codex exec` without disabling hooks and session persistence, the extractor spawns an infinite loop of hooks and pollutes `~/.claude/projects/` or `~/.codex/sessions/`! | None (plain HTTP completion). | None (plain HTTP completion). |

**Recommendation for `0x1-ai-memory`:**
1. **Default & Recommended Backend:** Pluggable OpenAI-compatible HTTP client supporting **direct API keys** (OpenRouter, Gemini, OpenAI, Anthropic) + **local Ollama/MLX** (plus local embeddings/FTS5 for zero-LLM-cost retrieval!).
2. **Optional Opt-In Experimental Backend (`provider: "cli-codex"` / `"cli-claude"`):** Only if explicitly selected by the user with a clear warning about rolling subscription quotas and ToS, strictly wrapped with `OX1_MEMORY_INTERNAL=1` recursion guard, `--no-session-persistence`/`--ephemeral`, `--tools ""`/`-s read-only`, disabled hooks, and disabled MCP servers.

---

## 6. Architectural Blueprint for `0x1-ai-memory` Across All 4 Clients

To achieve seamless, deduplicated memory across Claude Code, Codex CLI/App, Antigravity, and T3 Code with minimal surface area:

1. **Single Local Memory Daemon (Streamable HTTP MCP + Internal Hook API):**
   - Run one lightweight local daemon (e.g., `http://127.0.0.1:<port>`) exposing:
     - **Streamable HTTP MCP endpoint (`/mcp`)** — natively supported by **all three** harnesses (Claude Code `type: "http"`, Codex `url = "..."`, Antigravity `url: "..."`) without spawning 4–10 duplicate stdio processes or hitting macOS GUI `PATH` bugs! Provide a thin stdio fallback only if needed.
     - **Fast local hook REST endpoint (`/v1/hooks/recall`, `/v1/hooks/ingest`)** — returns pre-ranked memory context in <50ms (hybrid FTS5 + local vector search, **no LLM on the hot path**!) and queues asynchronous extraction jobs off-thread so synchronous hooks (especially Antigravity's blocking hooks) never stall the agent.
2. **Three Tiny Client Hook Shims (Same Daemon, Per-Client Envelope):**
   - **Claude Code (`~/.claude/settings.json`):** `SessionStart`, `UserPromptSubmit`, `PreCompact`, `Stop`, `SessionEnd` → emit `{ hookSpecificOutput: { hookEventName, additionalContext } }`. *(Automatically covers Claude inside T3 Code too!)*
   - **Codex CLI & Codex App (`~/.codex/hooks.json`):** `SessionStart`, `UserPromptSubmit`, `PreCompact`, `Stop`, `SessionEnd` → emit `{ hookSpecificOutput: { hookEventName, additionalContext } }` (**no `systemMessage`**). Keep the hook command static and instruct the user to approve once in `/hooks`. *(Automatically covers Codex inside T3 Code too!)*
   - **Antigravity (bundled plugin `~/.gemini/config/plugins/0x1-memory/`):** `plugin.json` + `mcp_config.json` + root `hooks.json` (`SessionStart`, `PreInvocation` [deduped per `lastUserInput`!], `Stop`) + `rules/AGENTS.md` → emit `{ injectSteps: [{ systemMessage: { systemMessage, metadata: { user_markdown } } }] }`.
3. **Optional Read-Only T3 Code Watcher (`~/.t3/userdata/statev2.sqlite`):**
   - Solves the **Antigravity-in-T3 isolation gap** (where T3 isolates `GEMINI_HOME` hooks/MCP) and captures any additional T3 providers (Cursor, Kiro, Pi, etc.) passively.
   - Deduplicate against native hook ingestion using `orchestration_v2_projection_provider_threads.provider_thread_id` (which holds the exact Claude `sessionId` / Codex `threadId` / Antigravity `conversationId`).
4. **Universal Recursion Guard:**
   - Every hook shim checks `if (process.env.OX1_MEMORY_INTERNAL === '1') process.exit(0);` as its very first line.

---

## Primary Sources & Verification Notes

- **Local shipped binaries & CLIs:**
  - `~/.local/bin/claude` (`2.1.331`: `--help`, `mcp --help`, `mcp add --help`)
  - `~/.local/bin/codex` (`codex-cli 0.172.0` Mach-O binary static strings, `--help`, `exec --help`, `mcp add --help`, `app-server --help`, `features list`)
  - `~/.local/bin/agy` (`--help`, `mcp --help`, `mcp add --help`, `plugin --help`)
- **Local Antigravity primary documentation:**
  - `~/.gemini/antigravity/builtin/skills/agy-customizations/SKILL.md`
  - `~/.gemini/antigravity/builtin/skills/agy-customizations/docs/{hooks,mcp_servers,rules,skills,plugins,json_configs}.md`
- **Local T3 Code (Beta `0.1.11`) shipped artifact (`app.asar`):**
  - `/Applications/T3 Code (Beta).app/Contents/Info.plist`
  - `app.asar:apps/server/dist/runtimeLayer-CyWe8jGq.mjs` (`src/provider/Drivers/ClaudeHome.ts`, `src/orchestration-v2/Adapters/ClaudeAdapterV2.ts`, `src/provider/Drivers/CodexHomeLayout.ts`, `codexAppServerArgs`, `codexAppServerEnvironment`)
  - `app.asar:apps/server/dist/server-8K5Ixyrs.mjs` (`src/provider/ClaudeProvider.ts`, `src/textGeneration/ClaudeTextGeneration.ts`, `src/textGeneration/CodexTextGeneration.ts`, `src/orchestration-v2/Adapters/CodexAdapterV2.ts`, `src/provider/CodexManagedHome.ts`, `src/provider/CodexManagedRuntime.ts`, `src/provider/antigravityAuthSupport.ts`, `src/orchestration-v2/Adapters/AntigravityAdapterV2.ts`, `src/mcp/McpHttpServer.ts`)
  - `app.asar:apps/server/dist/sdk-DZX4LfOe.mjs` (bundled `@anthropic-ai/claude-agent-sdk` CLI spawn flags, 33-event hook enum, `setMcpServers`)
- **Local non-secret schemas inspected:**
  - `~/.claude/settings.json` (redacted), `~/.claude.json` (keys/MCP names only), `~/.claude/projects/*/*.jsonl` (schema keys only)
  - `~/.codex/config.toml` (redacted sections), `~/.codex/hooks.json`, `~/.codex/.hook-trust/` (digest format), `~/.codex/{.codex-global-state,state_5,thread_history_1,memories_1,memories_v2_1}.sqlite` (DDL only), `~/.codex/sessions/*/*/*/*.jsonl` (schema keys only)
  - `~/.gemini/config/{mcp_config,hooks,config}.json` (redacted), `~/.gemini/antigravity/conversation_summaries.db` (DDL only), `~/.gemini/antigravity/brain/*/.system_generated/logs/transcript.jsonl` (schema keys only)
  - `~/.t3/userdata/{settings,desktop-settings,client-settings}.json` (redacted), `~/.t3/userdata/statev2.sqlite` (DDL only)
- **Official web documentation:**
  - Claude Code Hooks & MCP Reference (`code.claude.com`)
  - OpenAI Codex CLI, Hooks & Desktop App Configuration (`developers.openai.com/codex`)
  - Anthropic & OpenAI subscription OAuth vs API ToS and rate-limit policies

### Scope Limits & Unverified Items
- **Codex.app macOS binary:** Not installed in `/Applications` or `~/Applications` on this machine; Codex desktop app config-sharing facts rely on official OpenAI Codex documentation plus the observed `~/.codex/models_cache.codex_work_desktop.json` cache file rather than binary inspection of `Codex.app`.
- **Static vs. runtime verification for T3 Code:** T3 Code provider spawn behavior was established via verbatim static extraction from `app.asar` (`0.1.11`); live interactive threads inside T3 Code were not spawned to observe hook execution at runtime.
- **REA full JS graph:** `npx -y rea-agents@6.8.0 analyze-javascript-application` reported an unpacked native binary signature mismatch (`@clerk/electron-passkeys-darwin-arm64`) and hit the 3-minute MCP timeout under `--integrity-policy record-and-continue` due to bundle size (~18 MB of JS across `server`, `runtimeLayer`, `ManagedEndpointRuntime`, and `sdk`), so targeted read-only ASAR module extraction was used instead.
