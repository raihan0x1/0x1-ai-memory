# 0x1-ai-memory

Local-first, cross-agent persistent memory for coding agents, run as a single local binary with an embedded Web UI.

## Language

**Daemon**:
The local background process that hosts the HTTP API, MCP endpoint, live event stream, and embedded Web UI.
_Avoid_: Server, background worker, backend

**Onboarding**:
The guided first-run flow in the Web UI that walks a new user through connecting AI clients and choosing memory engine settings before entering the main dashboard.
_Avoid_: Setup wizard, first-run screen, intro flow

**CLI Setup**:
The terminal command for inspecting and wiring local AI clients without using the Web UI.
_Avoid_: Setup wizard, onboarding

**Port Mismatch**:
The state in which a connected AI client's saved configuration points to a different Daemon port than the currently active port.
_Avoid_: Outdated config, stale port

**Auto-Start**:
The opt-in OS login service configuration that starts the Daemon automatically after a machine reboot without blocking explicit stops.
_Avoid_: Startup script, background daemon, login hook

