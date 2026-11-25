# 0x1-ai-memory

[![CI](https://github.com/raihan0x1/0x1-ai-memory/actions/workflows/ci.yml/badge.svg)](https://github.com/raihan0x1/0x1-ai-memory/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-orange.svg)](LICENSE)

Local-first, anti-drain, cross-agent persistent memory engine shipped as a **single Rust binary (`0x1-memory`)** with an embedded localhost Web UI.

Built to connect **Claude Code**, **Codex CLI & App**, **Antigravity**, **T3 Code**, and any MCP client to one unified memory store — without burning coding subscriptions or requiring Python/Node/Bun/Chroma sidecars.

## Why `0x1-ai-memory`?

Inspired by (and engineered to fix the root causes audited in [`docs/research/claude-mem-analysis.md`](docs/research/claude-mem-analysis.md)):

- **Anti-drain by design (Hybrid Per-Turn):** Hooks capture raw events locally at **0 LLM tokens** (<10ms). Stateless distillation runs **once per turn (`Stop` / idle / `PreCompact`)** instead of per-`Read`/`Grep` $O(N^2)$ conversation replay.
- **Zero silent fallback:** Never dumps traffic onto your Claude/Codex/Antigravity coding subscription when an API fails. Hard daily token & USD caps + transparent `usage_ledger`.
- **Decoupled Memory Engine:** Default to **OpenRouter**, **Gemini**, **Ollama (local)**, **OpenAI-compatible**, or **Zero-LLM safe mode ($0)**. CLI subscription passthrough (`claude -p`, `codex exec`, `agy -p`) is strictly opt-in with `OX1_MEMORY_INTERNAL=1` recursion protection.
- **Single Rust binary (~9ms hooks, 0 external runtimes):** SQLite WAL + FTS5 (BM25) + `sqlite-vec` + lazy local ONNX (`fastembed`) + Streamable HTTP MCP + embedded React Web UI (`rust-embed`).
- **Ultra-lean recall:** ~300–500 token `SessionStart` briefing + **5 compact MCP tools** (`memory_search`, `memory_get`, `memory_timeline`, `memory_save`, `memory_update`).
- **Cross-agent out of the box:** Unified by normalized Git remote/repo across worktrees and all 4 clients, with per-project privacy controls (`full` | `local_only` | `paused`) and deterministic secret scrubbing.

## Quick Start

### Instant Try via `npx` (recommended — downloads native Rust binary, 0 runtime deps)

```bash
npx 0x1-ai-memory
```

Starts the local daemon (`http://127.0.0.1:41777`) and automatically opens the **Setup Wizard** in your browser. On first run, the thin launcher downloads and SHA-256 verifies the prebuilt Rust binary into `~/.0x1-memory/bin/` so all hooks and the daemon run natively in Rust (~9ms hooks).

```bash
npx 0x1-ai-memory setup    # terminal setup wizard (auto-detect & connect all clients)
npx 0x1-ai-memory status   # daemon health, connected clients, and today's spend
```

### Native Binary Installer (no Node required — macOS Apple Silicon / Linux x64)

```bash
curl -fsSL https://raw.githubusercontent.com/raihan0x1/0x1-ai-memory/main/install.sh | sh
0x1-memory
```

### Build from Source

```bash
git clone https://github.com/raihan0x1/0x1-ai-memory.git
cd 0x1-ai-memory

# 1. Build embedded Web UI
cd ui && pnpm install && pnpm build && cd ..

# 2. Build single Rust binary
cargo build --release
```

## CLI Commands

| Command | Description |
| :--- | :--- |
| `0x1-memory` | Default 1-command start (`serve --open`): starts daemon and opens Setup Wizard (first run) or Live Stream |
| `0x1-memory serve [--port 41777] [--open]` | Start local daemon (HTTP API + `/mcp` + SSE + embedded Web UI + async distillation + T3 watcher) |
| `0x1-memory setup [--client all\|claude-code\|codex\|antigravity\|t3-code] [--yes] [--disconnect]` | Guided auto-detect setup wizard with diff preview & automatic backups in `~/.0x1-memory/backups/` |
| `0x1-memory status` | Show daemon health, connected clients, memory totals, and today's token/USD usage |
| `0x1-memory hook <claude\|codex\|antigravity> [event]` | Ultra-fast (<10ms) lifecycle hook shim (fails open if daemon is stopped) |
| `0x1-memory mcp-stdio` | Optional stdio JSON-RPC MCP fallback for clients without Streamable HTTP |
| `0x1-memory search <query>` | Terminal hybrid search |

## Development & Releases

```bash
# Run all local guardrails (shell, asset parity, UI build, fmt, clippy, offline tests)
./scripts/check.sh

# Bump version, verify, tag vX.Y.Z, and push to trigger GitHub Release automatically
./scripts/release.sh            # interactive (current / patch / minor / major / custom)
./scripts/release.sh patch      # or: current | minor | major | 0.2.0 [--dry-run]
```

## Documentation

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — full architecture, SQLite schema, and 4-client wiring blueprint.
- [`docs/research/claude-mem-analysis.md`](docs/research/claude-mem-analysis.md) — primary-source audit of `thedotmack/claude-mem`.
- [`docs/research/client-integrations.md`](docs/research/client-integrations.md) — primary-source hooks/MCP & T3 Code (`app.asar`) reverse engineering.
