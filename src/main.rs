use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ox1_memory::config::{AppPaths, DEFAULT_PORT};
use ox1_memory::connectors::ConnectorManager;
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
use tokio::sync::broadcast;

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

#[derive(Subcommand)]
enum Commands {
    /// Start the local daemon (HTTP API + Streamable HTTP MCP + SSE + embedded Web UI)
    Serve {
        #[arg(long, env = "OX1_MEMORY_PORT", default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Open the localhost Web UI in the default browser on startup
        #[arg(long)]
        open: bool,
    },
    /// Guided setup wizard: auto-detect and connect Claude Code, Codex, Antigravity, & T3 Code
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
    /// Show daemon status, connected AI clients, memory count, and today's token/cost usage
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
    let command = cli.command.unwrap_or(Commands::Serve {
        port: DEFAULT_PORT,
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
        Commands::Serve { port, open } => run_serve(paths, port, open).await,
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
    let distill =
        spawn_distill_worker(db.clone(), embedder.clone(), llm.clone(), events_tx.clone());
    spawn_t3_watcher(db.clone(), distill.clone());

    let home = dirs::home_dir().context("Home directory unavailable")?;
    let bin_path = std::env::current_exe().unwrap_or_else(|_| "0x1-memory".into());
    let connectors = ConnectorManager::new(home, paths.backups_dir.clone(), bin_path, port);
    let inspected = connectors.inspect_all(&db).unwrap_or_default();
    let any_connected = inspected.iter().any(|c| c.connected);

    let state = AppState {
        db,
        embedder,
        llm,
        distill,
        connectors,
        events_tx,
        hook_state: HookState::default(),
        port,
    };

    let app = build_router(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let base_url = format!("http://127.0.0.1:{port}");
    let open_url = if any_connected {
        base_url.clone()
    } else {
        format!("{base_url}/connections")
    };

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(bind_err) => {
            if daemon_is_healthy(port).await {
                println!("0x1-ai-memory daemon is already running on {base_url}");
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
    let _ = std::fs::write(&paths.pid_path, std::process::id().to_string());

    println!("0x1-ai-memory daemon listening on {base_url}");
    println!("  Web UI : {open_url}");
    println!("  MCP    : {base_url}/mcp");
    println!("  DB     : {}", paths.db_path.display());

    if open_browser {
        open_in_browser(&open_url);
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Shutting down 0x1-ai-memory cleanly...");
        })
        .await?;

    let _ = std::fs::remove_file(&paths.pid_path);
    Ok(())
}

async fn daemon_is_healthy(port: u16) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
    else {
        return false;
    };
    client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
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
            "Disconnect Wizard"
        } else {
            "Setup Wizard"
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
    let mgr = ConnectorManager::new(home, paths.backups_dir, bin_path, port);
    let conns = mgr.inspect_all(&db)?;

    let daemon_up = reqwest::Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
        .ok()
        .map(|c| {
            let url = format!("http://127.0.0.1:{port}/health");
            (c, url)
        });

    let live = match daemon_up {
        Some((c, url)) => c
            .get(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false),
        None => false,
    };

    println!(
        "Daemon     : {} (http://127.0.0.1:{port})",
        if live { "RUNNING" } else { "STOPPED" }
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
    for c in conns {
        println!(
            "  - {:<16} detected={:<5} connected={:<5} | {}",
            c.display_name, c.detected, c.connected, c.status_note
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
    fn no_subcommand_defaults_to_none_for_one_command_serve_open() {
        let cli = Cli::try_parse_from(["0x1-memory"]).unwrap();
        assert!(cli.command.is_none());
    }
}
