# Coding Standards

Judgement-call invariants for code review in `0x1-ai-memory`. Mechanical rules (`clippy.toml`, `string_slice`, shell lint, asset parity, `<RimSelect />`) are enforced automatically by `./scripts/check.sh` and CI.

## 1. Non-reentrant SQLite `Mutex` discipline (`src/db.rs`)

`MemoryDb` guards a single `rusqlite::Connection` behind `std::sync::Mutex`. Never call another `&self` method that locks `self.conn` while a `MutexGuard` is already held (non-reentrant deadlock). Query helper logic directly on the already-locked `&Connection` instead.

## 2. Outbound privacy, scrubbing & budget choke-points

Every code path that sends text off-machine — LLM distillation (`src/llm.rs`), API embeddings (`src/embed.rs`), or opt-in CLI subprocesses — must:

- Run `scrub::redact_secrets` **before** persistence or dispatch.
- Respect project `privacy_mode == "local_only"` by checking `Settings::is_external_destination()` / `is_local_base_url()`.
- Enforce daily token & USD caps and record every attempt (including `budget_blocked` and `error`) in `usage_ledger`.
- Never silently fall back to another provider or coding-agent subscription when a configured provider fails.

## 3. Safe external client config mutation (`src/connectors.rs`)

Mutating external AI client configs (`~/.claude*`, `~/.codex*`, `~/.gemini*`, `~/.t3*`) must always:

- Create a timestamped backup before writing.
- Fail safely if an existing config file cannot be parsed as valid JSON/TOML — never overwrite corrupt user config with `{}`.
- Write atomically (write temp file in same directory, then rename).
- Strip or disable recursive hooks/MCP servers when invoking CLI providers (`OX1_MEMORY_INTERNAL=1`, `-c mcp_servers={}`).

## 4. Localhost-only HTTP boundary (`src/server.rs`)

The daemon binds exclusively to `127.0.0.1`. Keep both CORS origin filtering and the `Host`/`Origin` middleware strict to localhost so arbitrary browser pages cannot invoke `/api/*`, `/mcp`, or `/v1/hooks/*` via DNS rebinding or cross-origin requests.

## 5. Embedded Rim UI consistency (`ui/src/`)

- Every data panel handles all 4 states cleanly: `loading`, `empty`, `error`, and `full`.
- Parse backend timestamps as explicit UTC (`Z` suffix) before local formatting.
- All dropdown controls in `ui/src/pages/` use `<RimSelect />` (top-layer popover, WAI-ARIA listbox) instead of native `<select>`.

## 6. Async I/O & Supply-Chain Hardening (SonarCloud Quality Gate)

Enforced locally by `scripts/sonar-guard.py` (inside `./scripts/check.sh`, `.githooks/pre-push`, and CI) and verified against SonarCloud by `./scripts/sonar-gate.sh`:

- **No blocking I/O in async Rust (`rust:S7487`)**: Inside any Rust `async fn` (including `#[tokio::test]` integration tests), never call blocking `std::fs`, `std::process::Command`, or `std::thread::sleep` directly — use `tokio::fs`, `tokio::process::Command`, or `tokio::task::spawn_blocking`.
- **No loose POSIX permissions (`rust:S2612`)**: Never pass world-accessible or group-writable modes (`0o755`, `0o644`, `0o775`) to `Permissions::from_mode` / `set_mode`; use owner-only modes (`0o700` for executables/dirs, `0o600` for files).
- In shell installers (`install.sh`), enforce HTTPS on `curl` (`--proto '=https' --tlsv1.2`).
- In GitHub Actions workflows (`.github/workflows/`), pin third-party actions to full 40-char commit SHAs, scope `permissions: contents: write` to the specific job that needs it, and pass `--locked` to `cargo build --release`.
- In the Node launcher (`npm/bin/0x1-memory.mjs`), invoke archive tools (`tar`, `unzip`, `powershell.exe`) via fixed system directories instead of bare `PATH` lookup; in `ui/src/api.ts`, validate same-origin `/api/` paths before calling `fetch`.
