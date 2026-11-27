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

**Client**:
An AI coding agent whose hooks the Daemon receives and into which it injects memories, such as Claude Code, Codex or Antigravity.
_Avoid_: Provider, host, app

**Host**:
The application that launches a Client, such as T3 Code, Orca or a terminal; the Daemon records it but never captures from it directly.
_Avoid_: Client, harness, wrapper

**Active Session**:
A session that has not ended and has had activity within the last 30 minutes; any other unended session is idle.
_Avoid_: Open session, live session

**Turn End**:
The agent's final message at the close of one turn.
_Avoid_: Turn stop, Distill

**Distillation**:
The Daemon's background process that turns a session's captured activity into memories.
_Avoid_: Summarization, extraction

**Briefing**:
The set of memories the Daemon gives a new session before its first prompt.
_Avoid_: Session-start context, preamble

**Handoff**:
The part of a Briefing that tells a new session what other recent sessions in the same project were doing, and how long ago.
_Avoid_: Continuation, context transfer, resume

**Recall**:
One instance of the Daemon placing a stored memory into an agent's context, either as part of a Briefing or in response to a prompt.
_Avoid_: Injection, retrieval, hit

