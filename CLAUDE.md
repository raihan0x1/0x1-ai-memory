# 0x1-ai-memory

Single Rust binary (`0x1-memory`) with an embedded React/Vite Web UI (`ui/dist/`).

- **Build order**: Run `cd ui && pnpm build` before `cargo build` / `cargo test` after fresh clone or any `ui/` edit (`src/server.rs` embeds `ui/dist/` via `rust-embed`).
- **Verification**: Run `./scripts/check.sh` before committing, and activate tracked git hooks once per clone via `git config core.hooksPath .githooks`. After pushing to `main`, run `./scripts/sonar-gate.sh --wait 240` to verify SonarCloud Quality Gate on the exact pushed SHA.
- **Release**: Run `./scripts/release.sh` to run `check.sh`, push `main` and wait for SonarCloud Quality Gate before bumping version, tagging `vX.Y.Z`, and pushing to trigger GitHub Release (which also gates publish on `sonar-gate`).
- **Pointers**:
  - Changing client hooks or MCP configs → read `docs/research/client-integrations.md` (Codex `timeoutSec`/no-`systemMessage`, Antigravity `injectSteps`/`PreInvocation` dedup, why Antigravity inside T3 Code is not captured).
  - Changing schema, distillation, budget caps, or privacy → read `docs/ARCHITECTURE.md`.
  - Reviewing diffs or PRs → read `CODING_STANDARDS.md`.

## Agent skills

### Issue tracker

GitHub Issues on `raihan0x1/0x1-ai-memory` via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout (`GLOSSARY.md` and `docs/adr/` at the repo root). See `docs/agents/domain.md`.
