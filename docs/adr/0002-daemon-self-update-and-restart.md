# Daemon-owned self-update with pre-swap verification and graceful port handoff

We implement 1-click updates from the Web UI inside the **Daemon** itself: downloading the matching GitHub Release archive, verifying `SHA256SUMS.txt` and a `<temp_bin> --version` smoke check, swapping the binary in place, and handing off the active `--port` to the new **Daemon** process while caching release notes in SQLite for post-restart "What's new" display.

## Considered Options

- **Daemon-owned self-update vs notify-only / delegating to `install.sh` or `npm`**: Notify-only forces users out of the Web UI into a terminal; shelling out to `npm` or `curl | sh` breaks when the install method differs (`npx`, `install.sh`, or direct binary) and introduces fragile external runtime dependencies. Doing download, checksum verification, extraction, and atomic swap inside the Rust **Daemon** keeps one uniform, verified trust boundary across macOS, Linux, and Windows, with automatic fallback to manual update commands for dev builds (`target/debug`, `target/release`), read-only binaries, or unsupported architectures.
- **In-place binary swap vs side-by-side versioned directories**: Unix supports atomic `rename` over a running binary; Windows locks running executables, so the **Daemon** renames the active `.exe` to `.old` before moving the verified binary into place and cleans up `.old` on next startup. When running from the `npx` cache (`~/.0x1-memory/bin/`), the **Daemon** also updates `.version` so subsequent `npx 0x1-ai-memory` invocations do not downgrade it.
- **Graceful port handoff vs relying on Auto-Start supervision**: Because **Auto-Start** (`launchd` / `systemd --user`) intentionally does not force-restart on clean exit (ADR 0001), the updater spawns the new binary on the same `--port` (waiting briefly for the port to free while the old **Daemon** flushes its HTTP response and exits cleanly), then lets the Web UI poll `/health` until the new version answers and reloads the embedded bundle.

## Consequences

- Download URLs must stay pinned to the official `raihan0x1/0x1-ai-memory` GitHub Releases host/path and fail closed on any SHA-256 or `--version` smoke-check mismatch before touching the active binary.
- Periodic background checks (`/releases/latest`, ignoring prereleases) cache responses with `ETag` in SQLite `settings` and provide an opt-out toggle in Engine to preserve local-first / offline operation.
- Dev builds and read-only system installs (e.g. `/usr/local/bin` without write permission) cannot self-replace and gracefully degrade to copyable manual update instructions.
