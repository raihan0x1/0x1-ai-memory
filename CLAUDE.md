# 0x1-ai-memory

Single Rust binary (`0x1-memory`) with an embedded React/Vite Web UI (`ui/dist/`).

- **Build order**: Run `cd ui && pnpm build` before `cargo build` / `cargo test` after fresh clone or any `ui/` edit (`src/server.rs` embeds `ui/dist/` via `rust-embed`).
- **Verification**: Run `./scripts/check.sh` before committing.
- **Release**: Run `./scripts/release.sh` to bump version, run checks, tag `vX.Y.Z`, and push to trigger GitHub Release.
- **Pointers**:
  - Changing client hooks, MCP configs, or T3 watcher → read `docs/research/client-integrations.md` (Codex `timeoutSec`/no-`systemMessage`, Antigravity `injectSteps`/`PreInvocation` dedup, T3 `statev2.sqlite` dual-ID dedup).
  - Changing schema, distillation, budget caps, or privacy → read `docs/ARCHITECTURE.md`.
  - Reviewing diffs or PRs → read `CODING_STANDARDS.md`.

## Agent skills

### Issue tracker

GitHub Issues on `raihan0x1/0x1-ai-memory` via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout (`GLOSSARY.md` and `docs/adr/` at the repo root). See `docs/agents/domain.md`.
