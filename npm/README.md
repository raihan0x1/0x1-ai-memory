# 0x1-ai-memory

Local-first, anti-drain, cross-agent persistent memory engine connecting **Claude Code**, **Codex CLI & App**, **Antigravity**, **T3 Code**, and any MCP client.

## Instant Start

```bash
npx 0x1-ai-memory
```

On first run, this thin zero-dependency launcher downloads the verified native Rust binary (`0x1-memory`, SHA-256 checked) into `~/.0x1-memory/bin/`, starts the local daemon, and opens the Setup Wizard in your browser. All lifecycle hooks and the daemon run natively in Rust (~9ms hooks, zero Node/Python runtime overhead).

```bash
# Terminal setup wizard
npx 0x1-ai-memory setup

# Status & today's spend
npx 0x1-ai-memory status
```

Full documentation & source: https://github.com/raihan0x1/0x1-ai-memory
