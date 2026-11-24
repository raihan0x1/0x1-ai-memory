use crate::config::ensure_parent_dir;
use crate::db::{ClientHeartbeatMap, MemoryDb};
use crate::models::ClientConnection;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, value};

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionPreview {
    pub client_id: String,
    pub display_name: String,
    pub target_files: Vec<String>,
    pub summary: String,
    pub diff_preview: String,
    pub manual_step_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UniversalMcpSnippets {
    pub streamable_http_url: String,
    pub json_http: String,
    pub toml_http: String,
    pub json_stdio: String,
}

#[derive(Clone)]
pub struct ConnectorManager {
    pub home_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub bin_path: PathBuf,
    pub port: u16,
}

impl ConnectorManager {
    pub fn new(home_dir: PathBuf, backups_dir: PathBuf, bin_path: PathBuf, port: u16) -> Self {
        Self {
            home_dir,
            backups_dir,
            bin_path,
            port,
        }
    }

    pub fn mcp_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    pub fn bin_str(&self) -> String {
        self.bin_path.to_string_lossy().into_owned()
    }

    pub fn universal_snippets(&self) -> UniversalMcpSnippets {
        let url = self.mcp_url();
        let bin = self.bin_str();
        UniversalMcpSnippets {
            streamable_http_url: url.clone(),
            json_http: serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "0x1-memory": {
                        "type": "http",
                        "url": url
                    }
                }
            }))
            .unwrap_or_default(),
            toml_http: format!(
                "[mcp_servers.0x1_memory]\nurl = \"{}\"\ndefault_tools_approval_mode = \"auto\"\n",
                self.mcp_url()
            ),
            json_stdio: serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "0x1-memory": {
                        "command": bin,
                        "args": ["mcp-stdio", "--port", self.port.to_string()]
                    }
                }
            }))
            .unwrap_or_default(),
        }
    }

    pub fn inspect_all(&self, db: &MemoryDb) -> Result<Vec<ClientConnection>> {
        let heartbeats = db.get_client_heartbeats().unwrap_or_default();
        let settings = db.get_settings().unwrap_or_default();
        let mut out = vec![
            self.inspect_claude(&heartbeats),
            self.inspect_codex(&heartbeats),
            self.inspect_antigravity(&heartbeats),
            self.inspect_t3(&heartbeats, settings.t3_watcher_enabled),
        ];
        for c in &mut out {
            let _ = db.save_client_connection(c);
        }
        Ok(out)
    }

    pub fn preview_connect(&self, client_id: &str) -> Result<ConnectionPreview> {
        let bin = self.bin_str();
        let url = self.mcp_url();
        match client_id {
            "claude-code" => Ok(ConnectionPreview {
                client_id: "claude-code".into(),
                display_name: "Claude Code".into(),
                target_files: vec![
                    self.home_dir.join(".claude.json").display().to_string(),
                    self.home_dir
                        .join(".claude/settings.json")
                        .display()
                        .to_string(),
                ],
                summary: "Adds Streamable HTTP MCP server to ~/.claude.json and ultra-fast (<5ms) lifecycle hooks to ~/.claude/settings.json. Also auto-works inside T3 Code.".into(),
                diff_preview: format!(
                    "+ ~/.claude.json -> mcpServers[\"0x1-memory\"] = {{ \"type\": \"http\", \"url\": \"{url}\" }}\n\
                     + ~/.claude/settings.json -> hooks[SessionStart, UserPromptSubmit, PostToolUse, PreCompact, Stop, SessionEnd]\n\
                       command: \"{bin} hook claude <Event>\""
                ),
                manual_step_hint: None,
            }),
            "codex" => Ok(ConnectionPreview {
                client_id: "codex".into(),
                display_name: "Codex CLI & App".into(),
                target_files: vec![
                    self.home_dir
                        .join(".codex/config.toml")
                        .display()
                        .to_string(),
                    self.home_dir
                        .join(".codex/hooks.json")
                        .display()
                        .to_string(),
                ],
                summary: "Adds Streamable HTTP MCP server + read-tool auto-approval to ~/.codex/config.toml (preserving comments) and static hook shims to ~/.codex/hooks.json. Also auto-works inside T3 Code.".into(),
                diff_preview: format!(
                    "+ ~/.codex/config.toml -> [features] hooks = true\n\
                     + ~/.codex/config.toml -> [mcp_servers.0x1_memory] url = \"{url}\"\n\
                     + ~/.codex/hooks.json  -> static command \"{bin} hook codex\" on SessionStart/UserPromptSubmit/PostToolUse/PreCompact/Stop/SessionEnd"
                ),
                manual_step_hint: Some(
                    "After connecting, run `/hooks` once inside Codex to approve the 0x1-memory SHA-256 hook trust (one-time only).".into(),
                ),
            }),
            "antigravity" => Ok(ConnectionPreview {
                client_id: "antigravity".into(),
                display_name: "Antigravity".into(),
                target_files: vec![self
                    .home_dir
                    .join(".gemini/config/plugins/0x1-memory/")
                    .display()
                    .to_string()],
                summary: "Installs a single atomic Antigravity plugin bundling Streamable HTTP MCP, root hooks.json (camelCase injectSteps + PreInvocation dedup), and concise AGENTS.md rules.".into(),
                diff_preview: format!(
                    "+ ~/.gemini/config/plugins/0x1-memory/plugin.json\n\
                     + ~/.gemini/config/plugins/0x1-memory/mcp_config.json (url: \"{url}\")\n\
                     + ~/.gemini/config/plugins/0x1-memory/hooks.json (command: \"{bin} hook antigravity\")\n\
                     + ~/.gemini/config/plugins/0x1-memory/rules/AGENTS.md"
                ),
                manual_step_hint: Some(
                    "Restart or reload Antigravity session so the new 0x1-memory plugin hooks & MCP tools activate.".into(),
                ),
            }),
            "t3-code" => Ok(ConnectionPreview {
                client_id: "t3-code".into(),
                display_name: "T3 Code".into(),
                target_files: vec![self
                    .home_dir
                    .join(".t3/userdata/statev2.sqlite")
                    .display()
                    .to_string()],
                summary: "Enables read-only WAL watcher on ~/.t3/userdata/statev2.sqlite to capture Antigravity-in-T3 threads (where T3 isolates global GEMINI_HOME). Claude Code & Codex threads already inherit native hooks and are deduplicated automatically.".into(),
                diff_preview: "+ settings.t3_watcher_enabled = true (read-only mode=ro; never writes to T3's database)".into(),
                manual_step_hint: None,
            }),
            other => bail!("Unknown client_id: {other}"),
        }
    }

    pub fn connect(&self, client_id: &str, db: &MemoryDb) -> Result<ClientConnection> {
        let backup = match client_id {
            "claude-code" => self.connect_claude()?,
            "codex" => self.connect_codex()?,
            "antigravity" => self.connect_antigravity()?,
            "t3-code" => {
                let mut settings = db.get_settings()?;
                settings.t3_watcher_enabled = true;
                db.save_settings(&settings)?;
                None
            }
            other => bail!("Unknown client_id: {other}"),
        };

        let all = self.inspect_all(db)?;
        let mut conn = all
            .into_iter()
            .find(|c| c.client_id == client_id)
            .context("Client not found after connect")?;
        if backup.is_some() {
            conn.config_backup_path = backup;
            let _ = db.save_client_connection(&conn);
        }
        Ok(conn)
    }

    pub fn disconnect(&self, client_id: &str, db: &MemoryDb) -> Result<ClientConnection> {
        match client_id {
            "claude-code" => self.disconnect_claude()?,
            "codex" => self.disconnect_codex()?,
            "antigravity" => self.disconnect_antigravity()?,
            "t3-code" => {
                let mut settings = db.get_settings()?;
                settings.t3_watcher_enabled = false;
                db.save_settings(&settings)?;
            }
            other => bail!("Unknown client_id: {other}"),
        }
        let all = self.inspect_all(db)?;
        all.into_iter()
            .find(|c| c.client_id == client_id)
            .context("Client not found after disconnect")
    }

    // --- Claude Code ---

    fn inspect_claude(&self, hb: &ClientHeartbeatMap) -> ClientConnection {
        let claude_json = self.home_dir.join(".claude.json");
        let settings_json = self.home_dir.join(".claude/settings.json");
        let detected = self.home_dir.join(".claude").exists() || claude_json.exists();

        let mcp_ok = read_json(&claude_json)
            .ok()
            .and_then(|v| v.get("mcpServers")?.get("0x1-memory").cloned())
            .is_some();
        let hooks_ok = std::fs::read_to_string(&settings_json)
            .map(|s| s.contains("hook claude"))
            .unwrap_or(false);

        let (last_hb, last_ev, backup) =
            hb.get("claude-code").cloned().unwrap_or((None, None, None));

        ClientConnection {
            client_id: "claude-code".into(),
            display_name: "Claude Code".into(),
            detected,
            connected: mcp_ok && hooks_ok,
            needs_manual_step: false,
            manual_step_hint: None,
            config_paths: vec![
                claude_json.display().to_string(),
                settings_json.display().to_string(),
            ],
            config_backup_path: backup,
            last_heartbeat_at: last_hb,
            last_event_at: last_ev,
            status_note: if mcp_ok && hooks_ok {
                "Streamable HTTP MCP + 6 lifecycle hooks active (also inherited by T3 Code)".into()
            } else if detected {
                "Detected locally — ready to connect with 1 click".into()
            } else {
                "~/.claude not found on this machine".into()
            },
        }
    }

    fn connect_claude(&self) -> Result<Option<String>> {
        let claude_json = self.home_dir.join(".claude.json");
        let settings_json = self.home_dir.join(".claude/settings.json");
        let backup = self.backup_file(&claude_json)?;
        let _ = self.backup_file(&settings_json)?;

        // 1. ~/.claude.json -> mcpServers["0x1-memory"]
        let mut cj = read_json(&claude_json).unwrap_or_else(|_| json!({}));
        ensure_object(&mut cj);
        let servers = cj
            .as_object_mut()
            .unwrap()
            .entry("mcpServers")
            .or_insert_with(|| json!({}));
        ensure_object(servers);
        servers.as_object_mut().unwrap().insert(
            "0x1-memory".into(),
            json!({
                "type": "http",
                "url": self.mcp_url()
            }),
        );
        write_json(&claude_json, &cj)?;

        // 2. ~/.claude/settings.json -> hooks
        let mut sj = read_json(&settings_json).unwrap_or_else(|_| json!({}));
        ensure_object(&mut sj);
        let hooks = sj
            .as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert_with(|| json!({}));
        ensure_object(hooks);
        let hooks_map = hooks.as_object_mut().unwrap();

        let bin = self.bin_str();
        let events: [(&str, Option<&str>); 6] = [
            ("SessionStart", None),
            ("UserPromptSubmit", None),
            ("PostToolUse", Some("Edit|Write|MultiEdit|Bash")),
            ("PreCompact", None),
            ("Stop", None),
            ("SessionEnd", None),
        ];

        for (ev, matcher) in events {
            let entry = hooks_map.entry(ev).or_insert_with(|| json!([]));
            if !entry.is_array() {
                *entry = json!([]);
            }
            let arr = entry.as_array_mut().unwrap();
            // Idempotent: strip any existing 0x1-memory entries first
            arr.retain(|item| {
                !item.to_string().contains("0x1-memory")
                    && !item.to_string().contains("hook claude")
            });
            let cmd = format!("\"{bin}\" hook claude {ev} --port {}", self.port);
            let mut group = json!({
                "hooks": [{
                    "type": "command",
                    "command": cmd,
                    "timeout": 10
                }]
            });
            if let Some(m) = matcher {
                group["matcher"] = json!(m);
            }
            arr.push(group);
        }
        write_json(&settings_json, &sj)?;
        Ok(backup)
    }

    fn disconnect_claude(&self) -> Result<()> {
        let claude_json = self.home_dir.join(".claude.json");
        if claude_json.exists() {
            let _ = self.backup_file(&claude_json)?;
            if let Ok(mut cj) = read_json(&claude_json) {
                if let Some(servers) = cj.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
                    servers.remove("0x1-memory");
                }
                write_json(&claude_json, &cj)?;
            }
        }
        let settings_json = self.home_dir.join(".claude/settings.json");
        if settings_json.exists() {
            let _ = self.backup_file(&settings_json)?;
            if let Ok(mut sj) = read_json(&settings_json) {
                if let Some(hooks) = sj.get_mut("hooks").and_then(|v| v.as_object_mut()) {
                    for (_k, v) in hooks.iter_mut() {
                        if let Some(arr) = v.as_array_mut() {
                            arr.retain(|item| {
                                !item.to_string().contains("0x1-memory")
                                    && !item.to_string().contains("hook claude")
                            });
                        }
                    }
                }
                write_json(&settings_json, &sj)?;
            }
        }
        Ok(())
    }

    // --- Codex CLI & App ---

    fn inspect_codex(&self, hb: &ClientHeartbeatMap) -> ClientConnection {
        let config_toml = self.home_dir.join(".codex/config.toml");
        let hooks_json = self.home_dir.join(".codex/hooks.json");
        let detected = self.home_dir.join(".codex").exists();

        let toml_text = std::fs::read_to_string(&config_toml).unwrap_or_default();
        let mcp_ok = toml_text.contains("[mcp_servers.0x1_memory]");
        let hooks_ok = std::fs::read_to_string(&hooks_json)
            .map(|s| s.contains("hook codex"))
            .unwrap_or(false);
        let (last_hb, last_ev, backup) = hb.get("codex").cloned().unwrap_or((None, None, None));

        // Codex requires 1-time /hooks trust approval unless we've already observed live heartbeats
        let needs_manual = mcp_ok && hooks_ok && last_ev.is_none();

        ClientConnection {
            client_id: "codex".into(),
            display_name: "Codex CLI & App".into(),
            detected,
            connected: mcp_ok && hooks_ok,
            needs_manual_step: needs_manual,
            manual_step_hint: if needs_manual {
                Some("Run `/hooks` once inside Codex CLI/App to trust the static 0x1-memory hook command (SHA-256 trust gate).".into())
            } else {
                None
            },
            config_paths: vec![
                config_toml.display().to_string(),
                hooks_json.display().to_string(),
            ],
            config_backup_path: backup,
            last_heartbeat_at: last_hb,
            last_event_at: last_ev,
            status_note: if mcp_ok && hooks_ok {
                if needs_manual {
                    "Configured! Waiting for one-time `/hooks` approval in Codex.".into()
                } else {
                    "Streamable HTTP MCP + trusted hooks active (shared across Codex CLI, App & T3 Code)".into()
                }
            } else if detected {
                "Detected locally — ready to connect".into()
            } else {
                "~/.codex not found on this machine".into()
            },
        }
    }

    fn connect_codex(&self) -> Result<Option<String>> {
        let config_toml = self.home_dir.join(".codex/config.toml");
        let hooks_json = self.home_dir.join(".codex/hooks.json");
        let backup = self.backup_file(&config_toml)?;
        let _ = self.backup_file(&hooks_json)?;

        // 1. Format-preserving TOML update via toml_edit
        let existing_toml = std::fs::read_to_string(&config_toml).unwrap_or_default();
        let mut doc = existing_toml
            .parse::<DocumentMut>()
            .unwrap_or_else(|_| DocumentMut::new());

        if !doc.contains_key("features") {
            doc["features"] = Item::Table(Table::new());
        }
        doc["features"]["hooks"] = value(true);

        if !doc.contains_key("mcp_servers") {
            doc["mcp_servers"] = Item::Table(Table::new());
        }
        let mut srv = Table::new();
        srv["url"] = value(self.mcp_url());
        srv["default_tools_approval_mode"] = value("auto");

        let mut tools = Table::new();
        for read_tool in ["memory_search", "memory_get", "memory_timeline"] {
            let mut t = Table::new();
            t["approval_mode"] = value("approve");
            tools[read_tool] = Item::Table(t);
        }
        srv["tools"] = Item::Table(tools);
        doc["mcp_servers"]["0x1_memory"] = Item::Table(srv);

        ensure_parent_dir(&config_toml)?;
        std::fs::write(&config_toml, doc.to_string())?;

        // 2. ~/.codex/hooks.json (static command so SHA-256 trust never breaks!)
        let mut hj = read_json(&hooks_json).unwrap_or_else(|_| json!({}));
        ensure_object(&mut hj);
        let hooks = hj
            .as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert_with(|| json!({}));
        ensure_object(hooks);
        let hooks_map = hooks.as_object_mut().unwrap();

        let bin = self.bin_str();
        let static_cmd = format!("\"{bin}\" hook codex --port {}", self.port);
        let events: [(&str, Option<&str>); 6] = [
            ("SessionStart", None),
            ("UserPromptSubmit", None),
            (
                "PostToolUse",
                Some("shell|exec_command|apply_patch|write_file|edit_file"),
            ),
            ("PreCompact", None),
            ("Stop", None),
            ("SessionEnd", None),
        ];
        for (ev, matcher) in events {
            let entry = hooks_map.entry(ev).or_insert_with(|| json!([]));
            if !entry.is_array() {
                *entry = json!([]);
            }
            let arr = entry.as_array_mut().unwrap();
            arr.retain(|item| {
                !item.to_string().contains("0x1-memory") && !item.to_string().contains("hook codex")
            });
            let mut group = json!({
                "hooks": [{
                    "type": "command",
                    "command": static_cmd,
                    "timeout": 10
                }]
            });
            if let Some(m) = matcher {
                group["matcher"] = json!(m);
            }
            arr.push(group);
        }
        write_json(&hooks_json, &hj)?;
        Ok(backup)
    }

    fn disconnect_codex(&self) -> Result<()> {
        let config_toml = self.home_dir.join(".codex/config.toml");
        if config_toml.exists() {
            let _ = self.backup_file(&config_toml)?;
            if let Ok(raw) = std::fs::read_to_string(&config_toml)
                && let Ok(mut doc) = raw.parse::<DocumentMut>()
            {
                if let Some(tbl) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_mut()) {
                    tbl.remove("0x1_memory");
                }
                std::fs::write(&config_toml, doc.to_string())?;
            }
        }
        let hooks_json = self.home_dir.join(".codex/hooks.json");
        if hooks_json.exists() {
            let _ = self.backup_file(&hooks_json)?;
            if let Ok(mut hj) = read_json(&hooks_json) {
                if let Some(hooks) = hj.get_mut("hooks").and_then(|v| v.as_object_mut()) {
                    for (_k, v) in hooks.iter_mut() {
                        if let Some(arr) = v.as_array_mut() {
                            arr.retain(|item| {
                                !item.to_string().contains("0x1-memory")
                                    && !item.to_string().contains("hook codex")
                            });
                        }
                    }
                }
                write_json(&hooks_json, &hj)?;
            }
        }
        Ok(())
    }

    // --- Antigravity (Atomic Native Plugin) ---

    fn inspect_antigravity(&self, hb: &ClientHeartbeatMap) -> ClientConnection {
        let plugin_dir = self.home_dir.join(".gemini/config/plugins/0x1-memory");
        let detected = self.home_dir.join(".gemini").exists()
            || Path::new("/Applications/Antigravity.app").exists();
        let connected = plugin_dir.join("plugin.json").exists()
            && plugin_dir.join("hooks.json").exists()
            && plugin_dir.join("mcp_config.json").exists();
        let (last_hb, last_ev, backup) =
            hb.get("antigravity").cloned().unwrap_or((None, None, None));

        ClientConnection {
            client_id: "antigravity".into(),
            display_name: "Antigravity".into(),
            detected,
            connected,
            needs_manual_step: false,
            manual_step_hint: None,
            config_paths: vec![plugin_dir.display().to_string()],
            config_backup_path: backup,
            last_heartbeat_at: last_hb,
            last_event_at: last_ev,
            status_note: if connected {
                "Atomic plugin installed (Streamable HTTP MCP + root hooks.json + rules/AGENTS.md)"
                    .into()
            } else if detected {
                "Detected locally — installs as a clean, removable plugin".into()
            } else {
                "~/.gemini not found on this machine".into()
            },
        }
    }

    fn connect_antigravity(&self) -> Result<Option<String>> {
        let plugin_dir = self.home_dir.join(".gemini/config/plugins/0x1-memory");
        std::fs::create_dir_all(plugin_dir.join("rules"))?;
        let bin = self.bin_str();
        let cmd = format!("\"{bin}\" hook antigravity --port {}", self.port);

        write_json(
            &plugin_dir.join("plugin.json"),
            &json!({
                "name": "0x1-memory",
                "version": env!("CARGO_PKG_VERSION"),
                "description": "Cross-agent local-first AI memory (shared with Claude Code, Codex, and T3 Code)"
            }),
        )?;

        write_json(
            &plugin_dir.join("mcp_config.json"),
            &json!({
                "mcpServers": {
                    "0x1-memory": {
                        "url": self.mcp_url()
                    }
                }
            }),
        )?;

        // Verified Antigravity native schema: flat arrays for SessionStart/PreInvocation/Stop,
        // grouped {matcher, hooks} for PostToolUse.
        write_json(
            &plugin_dir.join("hooks.json"),
            &json!({
                "0x1-memory": {
                    "SessionStart": [
                        { "type": "command", "command": cmd, "timeout": 10 }
                    ],
                    "PreInvocation": [
                        { "type": "command", "command": cmd, "timeout": 10 }
                    ],
                    "PostToolUse": [
                        {
                            "matcher": "run_command|write_file|edit_file",
                            "hooks": [
                                { "type": "command", "command": cmd, "timeout": 10 }
                            ]
                        }
                    ],
                    "Stop": [
                        { "type": "command", "command": cmd, "timeout": 15 }
                    ]
                }
            }),
        )?;

        std::fs::write(
            plugin_dir.join("rules/AGENTS.md"),
            "# 0x1-ai-memory\n\n\
             You have shared cross-agent memory via the `0x1-memory` MCP server (shared with Claude Code, Codex, and T3 Code).\n\
             - Use `memory_search` to check prior architectural decisions or bugfixes before re-investigating.\n\
             - Use `memory_get([ids])` to load full details for IDs mentioned in the SessionStart briefing.\n\
             - Use `memory_save` when the user states a durable preference or makes a key architectural decision.\n",
        )?;

        Ok(Some(plugin_dir.display().to_string()))
    }

    fn disconnect_antigravity(&self) -> Result<()> {
        let plugin_dir = self.home_dir.join(".gemini/config/plugins/0x1-memory");
        if plugin_dir.exists() {
            std::fs::remove_dir_all(&plugin_dir)?;
        }
        Ok(())
    }

    // --- T3 Code ---

    fn inspect_t3(&self, hb: &ClientHeartbeatMap, watcher_enabled: bool) -> ClientConnection {
        let t3_db = self.home_dir.join(".t3/userdata/statev2.sqlite");
        let detected = t3_db.exists() || Path::new("/Applications/T3 Code (Beta).app").exists();
        let (last_hb, last_ev, backup) = hb.get("t3-code").cloned().unwrap_or((None, None, None));

        ClientConnection {
            client_id: "t3-code".into(),
            display_name: "T3 Code".into(),
            detected,
            connected: detected && watcher_enabled,
            needs_manual_step: false,
            manual_step_hint: None,
            config_paths: vec![t3_db.display().to_string()],
            config_backup_path: backup,
            last_heartbeat_at: last_hb,
            last_event_at: last_ev,
            status_note: if detected && watcher_enabled {
                "Read-only WAL watcher active for Antigravity-in-T3; Claude Code & Codex inherit native hooks automatically.".into()
            } else if detected {
                "Detected! Claude & Codex already pass through; enable watcher to capture isolated Antigravity-in-T3 threads.".into()
            } else {
                "~/.t3/userdata/statev2.sqlite not found on this machine".into()
            },
        }
    }

    fn backup_file(&self, src: &Path) -> Result<Option<String>> {
        if !src.exists() {
            return Ok(None);
        }
        std::fs::create_dir_all(&self.backups_dir)?;
        let fname = src
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config".into());
        let ts = Utc::now().format("%Y%m%d-%H%M%S");
        let dest = self.backups_dir.join(format!("{ts}-{fname}.bak"));
        std::fs::copy(src, &dest)?;
        Ok(Some(dest.display().to_string()))
    }
}

fn read_json(path: &Path) -> Result<Value> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn write_json(path: &Path, val: &Value) -> Result<()> {
    ensure_parent_dir(path)?;
    let pretty = serde_json::to_string_pretty(val)?;
    std::fs::write(path, format!("{pretty}\n"))?;
    Ok(())
}

fn ensure_object(val: &mut Value) {
    if !val.is_object() {
        *val = json!({});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn connect_and_disconnect_all_clients_idempotently_in_sandbox() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".gemini/config")).unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            "# keep my comment\nmodel = \"gpt-5\"\n",
        )
        .unwrap();

        let db = MemoryDb::open_in_memory().unwrap();
        let mgr = ConnectorManager::new(
            home.clone(),
            tmp.path().join("backups"),
            PathBuf::from("/usr/local/bin/0x1-memory"),
            41777,
        );

        // Connect Claude Code twice (idempotent, no duplicate hooks)
        let c1 = mgr.connect("claude-code", &db).unwrap();
        assert!(c1.connected);
        let c2 = mgr.connect("claude-code", &db).unwrap();
        assert!(c2.connected);
        let settings_raw = std::fs::read_to_string(home.join(".claude/settings.json")).unwrap();
        assert_eq!(settings_raw.matches("hook claude SessionStart").count(), 1);

        // Connect Codex (preserves TOML comments!)
        let codex = mgr.connect("codex", &db).unwrap();
        assert!(codex.connected);
        assert!(codex.needs_manual_step);
        let toml_raw = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(toml_raw.contains("# keep my comment"));
        assert!(toml_raw.contains("[mcp_servers.0x1_memory]"));

        // Connect Antigravity (atomic plugin)
        let agy = mgr.connect("antigravity", &db).unwrap();
        assert!(agy.connected);
        assert!(
            home.join(".gemini/config/plugins/0x1-memory/hooks.json")
                .exists()
        );

        // Disconnect all cleanly
        assert!(!mgr.disconnect("claude-code", &db).unwrap().connected);
        assert!(!mgr.disconnect("codex", &db).unwrap().connected);
        assert!(!mgr.disconnect("antigravity", &db).unwrap().connected);
    }
}
