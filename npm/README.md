# 0x1-ai-memory

Local-first, anti-drain, cross-agent persistent memory engine connecting **Claude Code**, **Codex CLI & App**, **Antigravity**, and any MCP client.

## Instant Start

```bash
npx 0x1-ai-memory
```

On first run, this thin zero-dependency launcher downloads the verified native Rust binary (`0x1-memory`, SHA-256 checked) into `~/.0x1-memory/bin/`, starts the local **Daemon** in the background on `:10101`, opens **Onboarding** in your browser, and returns your terminal prompt. All lifecycle hooks and the Daemon run natively in Rust (~9ms hooks, zero Node/Python runtime overhead).

```bash
# CLI Setup (terminal)
npx 0x1-ai-memory setup

# Daemon status, Auto-Start & today's spend
npx 0x1-ai-memory status

# Stop background Daemon cleanly
npx 0x1-ai-memory stop
```

Full documentation & source: https://github.com/raihan0x1/0x1-ai-memory
