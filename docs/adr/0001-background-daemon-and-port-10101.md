# Self-daemonizing background Daemon on port 10101

We changed the default Daemon port from `41777` to `10101` (the valid 16-bit TCP form of `010101`) and made the default `0x1-memory` / `npx 0x1-ai-memory` invocation self-daemonize in the background before opening the Web UI.

## Considered Options

- **Keep port `41777` vs switch to `10101`**: `41777` was arbitrary; `10101` is memorable, unprivileged, and matches the `0x1` brand. Because v0.1.2 already wrote `41777` into connected AI client configs, we pair the change with explicit Port Mismatch detection and 1-click reconnect rather than silently reporting broken clients as connected.
- **Foreground default vs self-daemonizing Rust binary**: Keeping `npx 0x1-ai-memory` attached to the terminal blocked the user's shell whenever the Web UI was open. Doing detachment inside the Rust binary (`serve --foreground` child + PID/log file in `~/.0x1-memory/`) keeps `npx`, `install.sh`, and direct binary runs identical across macOS, Linux, and Windows, while keeping OS login Auto-Start (`launchd` / `systemd --user`) strictly opt-in.

## Consequences

- Running `0x1-memory` or `npx 0x1-ai-memory` returns the terminal prompt immediately once `/health` succeeds; foreground debugging uses `0x1-memory serve --foreground`.
- Existing v0.1.2 client configs pointing to `:41777` surface as Port Mismatch (`connected = false`) until reconnected to `:10101`.
