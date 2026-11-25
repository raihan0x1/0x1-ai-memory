use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use ox1_memory::config::{AppPaths, DEFAULT_PORT};
use ox1_memory::connectors::ConnectorManager;
use ox1_memory::daemon::{
    AutoStartManager, daemon_is_healthy, daemon_needs_upgrade, fetch_daemon_health,
    fetch_onboarding_completed, format_pid_entry, read_log_tail, spawn_background, stop_daemon,
};
use ox1_memory::db::MemoryDb;
use ox1_memory::distill::spawn_distill_worker;
use ox1_memory::embed::Embedder;
use ox1_memory::hooks::HookState;
use ox1_memory::llm::MemoryLlmClient;
use ox1_memory::mcp::handle_rpc;
use ox1_memory::models::{SearchFilter, StreamEvent};
use ox1_memory::server::{AppState, build_router};
use ox1_memory::t3_watcher::spawn_t3_watcher;
use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::{broadcast, watch};

#[derive(Parser)]
#[command(
    name = "0x1-memory",
    version,
    about = "Local-first, anti-drain cross-agent AI memory (Claude Code, Codex, Antigravity, T3 Code)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum AutoStartAction {
    Enable,
    Disable,
    Status,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Daemon in the background (default when run with no subcommand)
    Start {
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Open the localhost Web UI in the default browser once healthy
        #[arg(long)]
        open: bool,
    },
    /// Stop the running background Daemon cleanly
    Stop {
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Restart the background Daemon
    Restart {
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Open the localhost Web UI in the default browser after restart
        #[arg(long)]
        open: bool,
    },
    /// Show recent Daemon log output from ~/.0x1-memory/daemon.log
    Logs {
        #[arg(long, short = 'n', default_value_t = 50)]
        lines: usize,
    },
    /// Manage opt-in OS login Auto-Start (macOS launchd / Linux systemd --user)
    Autostart {
        #[arg(value_enum)]
        action: AutoStartAction,
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Run the Daemon in the foreground (for debugging, launchd, or systemd)
    Serve {
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Open the localhost Web UI in the default browser on startup
        #[arg(long)]
        open: bool,
        /// Explicit foreground flag (default behavior of `serve`)
        #[arg(long, default_value_t = true)]
        foreground: bool,
    },
    /// CLI Setup: auto-detect and connect Claude Code, Codex, Antigravity, & T3 Code from terminal
    Setup {
        /// Target client (all, claude-code, codex, antigravity, t3-code)
        #[arg(long, default_value = "all")]
        client: String,
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Apply changes without interactive confirmation prompt
        #[arg(long)]
        yes: bool,
        /// Disconnect instead of connect
        #[arg(long)]
        disconnect: bool,
    },
    /// Show Daemon status, Auto-Start, connected AI clients, memory count, and today's usage
    Status {
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Ultra-fast (<5ms) lifecycle hook shim called by Claude Code, Codex, or Antigravity
    Hook {
        /// Client name (claude, codex, antigravity)
        client: String,
        /// Optional event name override (e.g. SessionStart, Stop)
        event: Option<String>,
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Optional stdio JSON-RPC MCP server fallback for clients without Streamable HTTP
    McpStdio {
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Quick hybrid search from the terminal
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

fn main() -> Result<()> {
    // UNIVERSAL RECURSION GUARD (<1ms): prevents infinite loops if opt-in CLI providers run.
    if std::env::var("OX1_MEMORY_INTERNAL").as_deref() == Ok("1") {
        println!("{{}}");
        return Ok(());
    }

    let cli = Cli::parse();
    let default_port = std::env::var("OX1_MEMORY_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let command = cli.command.unwrap_or(Commands::Start {
        port: default_port,
        open: true,
    });

    // Fast path for `hook`: zero tokio runtime overhead, pure blocking HTTP POST (<5ms)!
    if let Commands::Hook {
        client,
        event,
        port,
    } = command
    {
        return run_hook_fast(&client, event.as_deref(), port);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async_main(command))
}

/// Ultra-fast synchronous hook shim: reads stdin JSON, posts to local daemon with 1.5s timeout.
/// Fails open (`{}`) if daemon is stopped so coding agents are NEVER blocked.
fn run_hook_fast(client: &str, event: Option<&str>, port: u16) -> Result<()> {
    let mut stdin_buf = String::new();
    let _ = io::stdin().read_to_string(&mut stdin_buf);
    let payload: Value = if stdin_buf.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&stdin_buf).unwrap_or_else(|_| json!({ "raw": stdin_buf }))
    };

    let url = match event {
        Some(ev) if !ev.is_empty() => {
            format!("http://127.0.0.1:{port}/v1/hooks/{client}?event={ev}")
        }
        _ => format!("http://127.0.0.1:{port}/v1/hooks/{client}"),
    };

    let http = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_millis(250))
        .timeout(Duration::from_millis(1800))
        .build();

    if let Ok(c) = http
        && let Ok(resp) = c.post(&url).json(&payload).send()
        && resp.status().is_success()
        && let Ok(text) = resp.text()
        && !text.trim().is_empty()
    {
        println!("{text}");
        return Ok(());
    }

    println!("{{}}");
    Ok(())
}

async fn async_main(cmd: Commands) -> Result<()> {
    let paths = AppPaths::resolve()?;
    paths.ensure_dirs()?;

    match cmd {
        Commands::Start { port, open } => run_start(paths, port, open).await,
        Commands::Stop { port } => run_stop(paths, port).await,
        Commands::Restart { port, open } => {
            let _ = stop_daemon(&paths, port).await?;
            run_start(paths, port, open).await
        }
        Commands::Logs { lines } => run_logs(paths, lines),
        Commands::Autostart { action, port } => run_autostart(paths, action, port),
        Commands::Serve {
            port,
            open,
            foreground: _,
        } => run_serve(paths, port, open).await,
        Commands::Setup {
            client,
            port,
            yes,
            disconnect,
        } => run_setup(paths, &client, port, yes, disconnect),
        Commands::Status { port } => run_status(paths, port).await,
        Commands::McpStdio { port } => run_mcp_stdio(paths, port).await,
        Commands::Search { query, limit, port } => run_search(paths, &query, limit, port).await,
        Commands::Hook { .. } => unreachable!(),
    }
}

fn web_ui_url(base_url: &str, onboarding_done: bool) -> String {
    if onboarding_done {
        base_url.to_string()
    } else {
        format!("{base_url}/onboarding")
    }
}

async fn run_start(paths: AppPaths, port: u16, open_browser: bool) -> Result<()> {
    let base_url = format!("http://127.0.0.1:{port}");
    let current_ver = env!("CARGO_PKG_VERSION");
    if let Some(health) = fetch_daemon_health(port).await {
        if !daemon_needs_upgrade(health.version.as_deref(), current_ver) {
            let onboarding_done = fetch_onboarding_completed(port).await.unwrap_or(true);
            let open_url = web_ui_url(&base_url, onboarding_done);
            println!("0x1-ai-memory Daemon v{current_ver} is already running on {base_url}");
            println!("  Web UI : {open_url}");
            if open_browser {
                open_in_browser(&open_url);
            }
            return Ok(());
        }
        let old_ver = health.version.as_deref().unwrap_or("unknown");
        println!("Upgrading running Daemon on {base_url} (v{old_ver} -> v{current_ver})...");
        let _ = stop_daemon(&paths, port).await?;
        if daemon_is_healthy(port).await {
            anyhow::bail!(
                "Daemon v{old_ver} is still listening on port {port} after stop attempt; run `0x1-memory stop` or kill its PID and retry"
            );
        }
    }

    let bin_path =
        std::env::current_exe().context("Could not resolve 0x1-memory executable path")?;
    let pid = spawn_background(&paths, &bin_path, port).await?;
    let onboarding_done = fetch_onboarding_completed(port).await.unwrap_or(false);
    let open_url = web_ui_url(&base_url, onboarding_done);

    println!("0x1-ai-memory Daemon running in background (pid {pid})");
    println!("  Web UI : {open_url}");
    println!("  MCP    : {base_url}/mcp");
    println!("  DB     : {}", paths.db_path.display());
    println!("  Logs   : {}", paths.log_path.display());
    println!("  Stop   : 0x1-memory stop");

    if open_browser {
        open_in_browser(&open_url);
    }
    Ok(())
}

async fn run_stop(paths: AppPaths, port: u16) -> Result<()> {
    if stop_daemon(&paths, port).await? {
        println!("0x1-ai-memory Daemon stopped (port {port}).");
    } else {
        println!("0x1-ai-memory Daemon is not running on port {port}.");
    }
    Ok(())
}

fn run_logs(paths: AppPaths, lines: usize) -> Result<()> {
    let tail = read_log_tail(&paths.log_path, lines)?;
    if tail.is_empty() {
        println!("No Daemon logs found at {}.", paths.log_path.display());
    } else {
        println!("{tail}");
    }
    Ok(())
}

fn run_autostart(paths: AppPaths, action: AutoStartAction, port: u16) -> Result<()> {
    let home = dirs::home_dir().context("Home directory unavailable")?;
    let bin_path = std::env::current_exe().unwrap_or_else(|_| "0x1-memory".into());
    let mgr = AutoStartManager::for_current_os(home, bin_path, paths.log_path, port);
    let status = match action {
        AutoStartAction::Enable => mgr.enable()?,
        AutoStartAction::Disable => mgr.disable()?,
        AutoStartAction::Status => mgr.status(),
    };
    println!(
        "Auto-Start : {} (supported={})",
        if status.enabled {
            "ENABLED"
        } else {
            "DISABLED"
        },
        status.supported
    );
    if let Some(p) = status.service_path {
        println!("Service    : {p}");
    }
    println!("Note       : {}", status.note);
    Ok(())
}

async fn run_serve(paths: AppPaths, port: u16, open_browser: bool) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ox1_memory=info,tower_http=warn".into()),
        )
        .init();

    let db = MemoryDb::open(&paths.db_path)?;
    let embedder = Embedder::new(paths.models_dir.clone());
    let settings = db.get_settings().unwrap_or_default();
    if settings.embedding_mode == "local_onnx" {
        embedder.warm_local_async();
    }

    let llm = MemoryLlmClient::new();
    let (events_tx, _) = broadcast::channel::<StreamEvent>(256);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let distill =
        spawn_distill_worker(db.clone(), embedder.clone(), llm.clone(), events_tx.clone());
    spawn_t3_watcher(db.clone(), distill.clone());

    let home = dirs::home_dir().context("Home directory unavailable")?;
    let bin_path = std::env::current_exe().unwrap_or_else(|_| "0x1-memory".into());
    let connectors = ConnectorManager::new(
        home.clone(),
        paths.backups_dir.clone(),
        bin_path.clone(),
        port,
    );
    let autostart = AutoStartManager::for_current_os(home, bin_path, paths.log_path.clone(), port);

    // Seed onboarding BEFORE inspect_all saves client_connections so fresh T3 installs aren't misread as upgrades.
    let onboarding_done = db
        .seed_onboarding_if_existing(connectors.has_explicit_client_config())
        .unwrap_or(false);
    let _ = connectors.inspect_all(&db);

    let state = AppState {
        db,
        embedder,
        llm,
        distill,
        connectors,
        autostart,
        events_tx,
        shutdown_tx: shutdown_tx.clone(),
        hook_state: HookState::default(),
        port,
        data_dir: paths.data_dir.clone(),
        log_path: paths.log_path.clone(),
    };

    let app = build_router(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let base_url = format!("http://127.0.0.1:{port}");
    let open_url = web_ui_url(&base_url, onboarding_done);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(bind_err) => {
            if let Some(health) = fetch_daemon_health(port).await {
                if daemon_needs_upgrade(health.version.as_deref(), env!("CARGO_PKG_VERSION")) {
                    let old_ver = health.version.as_deref().unwrap_or("unknown");
                    anyhow::bail!(
                        "Another 0x1-ai-memory Daemon (v{old_ver}) is already listening on {base_url} while this binary is v{}. Run `0x1-memory restart` to reload.",
                        env!("CARGO_PKG_VERSION")
                    );
                }
                println!("0x1-ai-memory Daemon is already running on {base_url}");
                if open_browser {
                    open_in_browser(&open_url);
                }
                return Ok(());
            }
            return Err(bind_err).with_context(|| {
                format!("Failed to bind {addr} (port {port} is in use by another process)")
            });
        }
    };
    let _ = tokio::fs::write(&paths.pid_path, format_pid_entry(std::process::id(), port)).await;

    println!("0x1-ai-memory Daemon listening on {base_url}");
    println!("  Web UI : {open_url}");
    println!("  MCP    : {base_url}/mcp");
    println!("  DB     : {}", paths.db_path.display());

    if open_browser {
        open_in_browser(&open_url);
    }

    let sig_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        tracing::info!("Shutting down 0x1-ai-memory cleanly...");
        let _ = sig_tx.send(true);
    });

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while !*shutdown_rx.borrow() {
                if shutdown_rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await?;

    let _ = tokio::fs::remove_file(&paths.pid_path).await;
    Ok(())
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                if let Some(ref mut s) = sigterm {
                    s.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

fn run_setup(
    paths: AppPaths,
    client_arg: &str,
    port: u16,
    yes: bool,
    disconnect: bool,
) -> Result<()> {
    let db = MemoryDb::open(&paths.db_path)?;
    let home = dirs::home_dir().context("Home directory unavailable")?;
    let bin_path = std::env::current_exe().unwrap_or_else(|_| "0x1-memory".into());
    let mgr = ConnectorManager::new(home, paths.backups_dir.clone(), bin_path, port);

    let conns = mgr.inspect_all(&db)?;
    let targets: Vec<String> = if client_arg == "all" {
        conns
            .iter()
            .filter(|c| c.detected)
            .map(|c| c.client_id.clone())
            .collect()
    } else {
        vec![client_arg.to_string()]
    };

    if targets.is_empty() {
        println!("No supported AI clients detected locally. Use Universal MCP URL:");
        println!("  {}", mgr.mcp_url());
        return Ok(());
    }

    println!(
        "=== 0x1-ai-memory {} ===\n",
        if disconnect {
            "CLI Disconnect"
        } else {
            "CLI Setup"
        }
    );
    for cid in &targets {
        if let Ok(preview) = mgr.preview_connect(cid) {
            if disconnect {
                println!(
                    "[{}] Will remove 0x1-memory MCP/hooks entries (leaving all other settings intact):",
                    preview.display_name
                );
                for f in &preview.target_files {
                    println!("  - {f}");
                }
                println!();
            } else {
                println!("[{}] {}", preview.display_name, preview.summary);
                println!("{}\n", preview.diff_preview);
            }
        }
    }

    if !yes {
        print!(
            "{} {} client(s)? Backups will be saved to {} [y/N]: ",
            if disconnect { "Disconnect" } else { "Connect" },
            targets.len(),
            paths.backups_dir.display()
        );
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Aborted. No files were changed.");
            return Ok(());
        }
    }

    for cid in &targets {
        let res = if disconnect {
            mgr.disconnect(cid, &db)?
        } else {
            mgr.connect(cid, &db)?
        };
        println!(
            "✓ {:<16} connected={} — {}",
            res.display_name, res.connected, res.status_note
        );
        if let Some(hint) = res.manual_step_hint {
            println!("  ↳ Action needed: {hint}");
        }
    }

    Ok(())
}

async fn run_status(paths: AppPaths, port: u16) -> Result<()> {
    let db = MemoryDb::open(&paths.db_path)?;
    let settings = db.get_settings()?;
    let summary = db.usage_summary(&settings, settings.embedding_mode.clone())?;
    let home = dirs::home_dir().context("Home directory unavailable")?;
    let bin_path = std::env::current_exe().unwrap_or_else(|_| "0x1-memory".into());
    let mgr = ConnectorManager::new(home.clone(), paths.backups_dir, bin_path.clone(), port);
    let autostart = AutoStartManager::for_current_os(home, bin_path, paths.log_path, port).status();
    let conns = mgr.inspect_all(&db)?;

    let health = fetch_daemon_health(port).await;
    let current_ver = env!("CARGO_PKG_VERSION");
    let daemon_label = match health {
        Some(ref h) if daemon_needs_upgrade(h.version.as_deref(), current_ver) => {
            let old = h.version.as_deref().unwrap_or("unknown");
            format!(
                "RUNNING v{old} [OUTDATED: binary is v{current_ver} — run `0x1-memory restart`]"
            )
        }
        Some(ref h) => format!("RUNNING v{}", h.version.as_deref().unwrap_or(current_ver)),
        None => "STOPPED".to_string(),
    };

    println!("Daemon     : {daemon_label} (http://127.0.0.1:{port})");
    println!(
        "Auto-Start : {}",
        if autostart.enabled {
            "ENABLED"
        } else if autostart.supported {
            "DISABLED (run `0x1-memory autostart enable`)"
        } else {
            "MANUAL"
        }
    );
    println!(
        "Engine     : {} / {} (Zero silent fallback: ON)",
        summary.active_provider, summary.active_model
    );
    println!(
        "Memories   : {} active across {} project(s) | {} pending raw events",
        summary.total_memories, summary.total_projects, summary.pending_raw_events
    );
    println!(
        "Today Cost : {} tokens / ${:.4} (cap: {} tok / ${:.2})",
        summary.today_total_tokens,
        summary.today_cost_usd,
        summary.daily_token_cap,
        summary.daily_cost_cap_usd
    );
    println!("\nClients:");
    let mut has_mismatch = false;
    for c in conns {
        if c.port_mismatch {
            has_mismatch = true;
        }
        println!(
            "  - {:<16} detected={:<5} connected={:<5}{} | {}",
            c.display_name,
            c.detected,
            c.connected,
            if c.port_mismatch {
                " [PORT MISMATCH]"
            } else {
                ""
            },
            c.status_note
        );
    }
    if has_mismatch {
        println!(
            "\nAction needed: run `0x1-memory setup` or open http://127.0.0.1:{port}/connections to reconnect mismatched clients."
        );
    }
    Ok(())
}

async fn run_mcp_stdio(paths: AppPaths, port: u16) -> Result<()> {
    let db = MemoryDb::open(&paths.db_path)?;
    let embedder = Embedder::new(paths.models_dir);
    let (events_tx, _) = broadcast::channel::<StreamEvent>(32);
    let daemon_url = format!("http://127.0.0.1:{port}/mcp");
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(req): Result<Value, _> = serde_json::from_str(trimmed) else {
            continue;
        };
        // Forward to live daemon if running (so SSE UI updates stay live), else execute directly on SQLite
        let resp = match http.post(&daemon_url).json(&req).send().await {
            Ok(r) if r.status().is_success() => r.json::<Value>().await.ok(),
            _ => handle_rpc(req, &db, &embedder, &events_tx, Some("mcp-stdio")).await,
        };
        if let Some(reply) = resp {
            writeln!(stdout, "{}", serde_json::to_string(&reply)?)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

async fn run_search(paths: AppPaths, query: &str, limit: usize, port: u16) -> Result<()> {
    // Prefer running daemon for instant hybrid FTS5 + ONNX vector search; fallback to direct SQLite FTS5.
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(250))
        .timeout(Duration::from_secs(3))
        .build()
        .ok();
    if let Some(client) = http
        && let Ok(resp) = client
            .get(format!("http://127.0.0.1:{port}/api/memories"))
            .query(&[("query", query), ("limit", &limit.to_string())])
            .send()
            .await
        && resp.status().is_success()
        && let Ok(val) = resp.json::<Value>().await
        && let Some(arr) = val.get("memories").and_then(|v| v.as_array())
    {
        if arr.is_empty() {
            println!("No matching memories.");
            return Ok(());
        }
        for h in arr {
            println!(
                "[{}|{}|{}] {} — {} ({})",
                h["source_client"].as_str().unwrap_or("?"),
                h["memory_type"].as_str().unwrap_or("?"),
                h["status"].as_str().unwrap_or("?"),
                h["title"].as_str().unwrap_or(""),
                h["summary"].as_str().unwrap_or(""),
                h["id"].as_str().unwrap_or(""),
            );
        }
        return Ok(());
    }

    let db = MemoryDb::open(&paths.db_path)?;
    let hits = db.search_memories(
        &SearchFilter {
            query: Some(query.to_string()),
            limit: Some(limit),
            ..Default::default()
        },
        None,
    )?;
    if hits.is_empty() {
        println!("No matching memories.");
        return Ok(());
    }
    for h in hits {
        println!(
            "[{}|{}|{}] {} — {} ({})",
            h.source_client, h.memory_type, h.status, h.title, h.summary, h.id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_subcommand_defaults_to_none_for_one_command_background_start() {
        let cli = Cli::try_parse_from(["0x1-memory"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_daemon_lifecycle_and_autostart_subcommands_with_port_10101() {
        assert!(matches!(
            Cli::try_parse_from(["0x1-memory", "start"])
                .unwrap()
                .command,
            Some(Commands::Start {
                port: 10101,
                open: false
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["0x1-memory", "stop"]).unwrap().command,
            Some(Commands::Stop { port: 10101 })
        ));
        assert!(matches!(
            Cli::try_parse_from(["0x1-memory", "restart", "--open"])
                .unwrap()
                .command,
            Some(Commands::Restart {
                port: 10101,
                open: true
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["0x1-memory", "logs", "-n", "25"])
                .unwrap()
                .command,
            Some(Commands::Logs { lines: 25 })
        ));
        assert!(matches!(
            Cli::try_parse_from(["0x1-memory", "serve", "--foreground"])
                .unwrap()
                .command,
            Some(Commands::Serve {
                port: 10101,
                open: false,
                foreground: true
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["0x1-memory", "autostart", "enable"])
                .unwrap()
                .command,
            Some(Commands::Autostart {
                action: AutoStartAction::Enable,
                port: 10101
            })
        ));
    }
}
