#![deny(clippy::string_slice)]

use crate::config::{AppPaths, DEFAULT_PORT, write_atomic_file};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const MACOS_LAUNCHD_LABEL: &str = "com.0x1.memory";
pub const LINUX_SYSTEMD_UNIT: &str = "0x1-memory.service";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoStartPlatform {
    MacOs,
    Linux,
    Windows,
    Unsupported,
}

impl AutoStartPlatform {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Self::Unsupported
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoStartStatus {
    pub supported: bool,
    pub enabled: bool,
    pub platform: AutoStartPlatform,
    pub service_path: Option<String>,
    pub note: String,
}

#[derive(Clone)]
pub struct AutoStartManager {
    pub home_dir: PathBuf,
    pub bin_path: PathBuf,
    pub log_path: PathBuf,
    pub port: u16,
    pub platform: AutoStartPlatform,
    invoke_os_tools: bool,
}

impl AutoStartManager {
    pub fn for_current_os(
        home_dir: PathBuf,
        bin_path: PathBuf,
        log_path: PathBuf,
        port: u16,
    ) -> Self {
        Self {
            home_dir,
            bin_path,
            log_path,
            port,
            platform: AutoStartPlatform::current(),
            invoke_os_tools: true,
        }
    }

    pub fn for_testing(
        home_dir: PathBuf,
        bin_path: PathBuf,
        log_path: PathBuf,
        port: u16,
        platform: AutoStartPlatform,
    ) -> Self {
        Self {
            home_dir,
            bin_path,
            log_path,
            port,
            platform,
            invoke_os_tools: false,
        }
    }

    pub fn service_file_path(&self) -> Option<PathBuf> {
        match self.platform {
            AutoStartPlatform::MacOs => Some(
                self.home_dir
                    .join("Library/LaunchAgents")
                    .join(format!("{MACOS_LAUNCHD_LABEL}.plist")),
            ),
            AutoStartPlatform::Linux => Some(
                self.home_dir
                    .join(".config/systemd/user")
                    .join(LINUX_SYSTEMD_UNIT),
            ),
            AutoStartPlatform::Windows | AutoStartPlatform::Unsupported => None,
        }
    }

    pub fn status(&self) -> AutoStartStatus {
        match self.platform {
            AutoStartPlatform::MacOs => {
                let path = self.service_file_path().unwrap();
                let enabled = path.exists();
                AutoStartStatus {
                    supported: true,
                    enabled,
                    platform: self.platform,
                    service_path: Some(path.display().to_string()),
                    note: if enabled {
                        "Enabled via macOS launchd LaunchAgent (restarts only on crash; stays stopped after `0x1-memory stop`).".into()
                    } else {
                        "Opt-in macOS launchd LaunchAgent (starts Daemon automatically on login).".into()
                    },
                }
            }
            AutoStartPlatform::Linux => {
                let path = self.service_file_path().unwrap();
                let enabled = path.exists();
                AutoStartStatus {
                    supported: true,
                    enabled,
                    platform: self.platform,
                    service_path: Some(path.display().to_string()),
                    note: if enabled {
                        "Enabled via Linux systemd --user service (Restart=on-failure).".into()
                    } else {
                        "Opt-in Linux systemd --user service (starts Daemon automatically on login).".into()
                    },
                }
            }
            AutoStartPlatform::Windows => AutoStartStatus {
                supported: false,
                enabled: false,
                platform: self.platform,
                service_path: None,
                note: "Automatic login startup is currently supported on macOS and Linux. On Windows, run `0x1-memory` after login or add a shortcut in shell:startup.".into(),
            },
            AutoStartPlatform::Unsupported => AutoStartStatus {
                supported: false,
                enabled: false,
                platform: self.platform,
                service_path: None,
                note: "Automatic login startup is not supported on this operating system.".into(),
            },
        }
    }

    pub fn set_enabled(&self, enable: bool) -> Result<AutoStartStatus> {
        if enable {
            self.enable()
        } else {
            self.disable()
        }
    }

    pub fn enable(&self) -> Result<AutoStartStatus> {
        match self.platform {
            AutoStartPlatform::MacOs => {
                let path = self.service_file_path().unwrap();
                let plist = self.render_macos_plist();
                write_atomic_file(&path, &plist)?;
                if self.invoke_os_tools
                    && let Some(uid) = current_unix_uid()
                {
                    let _ = Command::new("launchctl")
                        .args(["enable", &format!("gui/{uid}/{MACOS_LAUNCHD_LABEL}")])
                        .status();
                }
                Ok(self.status())
            }
            AutoStartPlatform::Linux => {
                let path = self.service_file_path().unwrap();
                let unit = self.render_linux_unit();
                write_atomic_file(&path, &unit)?;
                if self.invoke_os_tools {
                    let _ = Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .status();
                    let status = Command::new("systemctl")
                        .args(["--user", "enable", LINUX_SYSTEMD_UNIT])
                        .status()
                        .inspect_err(|_| {
                            let _ = std::fs::remove_file(&path);
                        })
                        .context("Failed to run `systemctl --user enable`")?;
                    if !status.success() {
                        let _ = std::fs::remove_file(&path);
                        bail!(
                            "Failed to enable Auto-Start via `systemctl --user enable` (exit status {status})"
                        );
                    }
                }
                Ok(self.status())
            }
            AutoStartPlatform::Windows | AutoStartPlatform::Unsupported => {
                bail!("Automatic login startup is currently supported on macOS and Linux only.")
            }
        }
    }

    pub fn disable(&self) -> Result<AutoStartStatus> {
        match self.platform {
            AutoStartPlatform::MacOs => {
                let path = self.service_file_path().unwrap();
                if self.invoke_os_tools
                    && let Some(uid) = current_unix_uid()
                {
                    let _ = Command::new("launchctl")
                        .args(["disable", &format!("gui/{uid}/{MACOS_LAUNCHD_LABEL}")])
                        .status();
                }
                if path.exists() {
                    std::fs::remove_file(&path)?;
                }
                Ok(self.status())
            }
            AutoStartPlatform::Linux => {
                let path = self.service_file_path().unwrap();
                if self.invoke_os_tools {
                    let _ = Command::new("systemctl")
                        .args(["--user", "disable", LINUX_SYSTEMD_UNIT])
                        .status();
                }
                if path.exists() {
                    std::fs::remove_file(&path)?;
                }
                if self.invoke_os_tools {
                    let _ = Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .status();
                }
                Ok(self.status())
            }
            AutoStartPlatform::Windows | AutoStartPlatform::Unsupported => Ok(self.status()),
        }
    }

    pub fn render_macos_plist(&self) -> String {
        let bin = escape_xml(&self.bin_path.to_string_lossy());
        let log = escape_xml(&self.log_path.to_string_lossy());
        let port = self.port;
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{MACOS_LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{bin}</string>
    <string>serve</string>
    <string>--foreground</string>
    <string>--port</string>
    <string>{port}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#
        )
    }

    pub fn render_linux_unit(&self) -> String {
        let bin = self
            .bin_path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let port = self.port;
        format!(
            "[Unit]\n\
             Description=0x1-ai-memory local persistent AI memory daemon\n\
             After=network.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             ExecStart=\"{bin}\" serve --foreground --port {port}\n\
             Restart=on-failure\n\
             RestartSec=3\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n"
        )
    }
}

pub async fn daemon_is_healthy(port: u16) -> bool {
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

pub async fn fetch_onboarding_completed(port: u16) -> Option<bool> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .ok()?;
    let val: serde_json::Value = client
        .get(format!("http://127.0.0.1:{port}/api/settings"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    val.get("onboarding_completed")?.as_bool()
}

/// Spawns `0x1-memory serve --foreground --port <port>` detached from the terminal
/// and waits until `/health` responds OK.
pub async fn spawn_background(paths: &AppPaths, bin_path: &Path, port: u16) -> Result<u32> {
    paths.ensure_dirs()?;
    let stdout_file: File = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log_path)
        .with_context(|| format!("Failed to open log file {}", paths.log_path.display()))?;
    let stderr_file = stdout_file
        .try_clone()
        .context("Failed to clone daemon log handle")?;

    let mut cmd = Command::new(bin_path);
    cmd.args(["serve", "--foreground", "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Detach from caller's process group so closing the terminal or Ctrl-C won't kill the Daemon.
        cmd.process_group(0);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn background Daemon from {}",
            bin_path.display()
        )
    })?;
    let pid = child.id();

    for _ in 0..100 {
        if daemon_is_healthy(port).await {
            return Ok(pid);
        }
        if let Ok(Some(status)) = child.try_wait() {
            let tail = read_log_tail(&paths.log_path, 20).unwrap_or_default();
            bail!(
                "Daemon exited early with {status}. Recent log ({}):\n{tail}",
                paths.log_path.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let tail = read_log_tail(&paths.log_path, 20).unwrap_or_default();
    bail!(
        "Timed out waiting for Daemon on http://127.0.0.1:{port}/health. Recent log ({}):\n{tail}",
        paths.log_path.display()
    );
}

/// Stops a running Daemon cleanly via HTTP `/api/daemon/stop`, falling back to PID termination.
/// Returns `Ok(true)` if a running Daemon was stopped, `Ok(false)` if none was running.
pub async fn stop_daemon(paths: &AppPaths, port: u16) -> Result<bool> {
    let mut stopped_any = false;

    if daemon_is_healthy(port).await {
        stopped_any = true;
        if let Ok(client) = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
        {
            let _ = client
                .post(format!("http://127.0.0.1:{port}/api/daemon/stop"))
                .send()
                .await;
        }
        for _ in 0..30 {
            if !daemon_is_healthy(port).await {
                let _ = std::fs::remove_file(&paths.pid_path);
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    if let Ok(raw_pid) = std::fs::read_to_string(&paths.pid_path)
        && let Some((pid, pid_port)) = parse_pid_entry(&raw_pid)
        && pid_port == port
    {
        if signal_pid_terminate(pid) {
            stopped_any = true;
            for _ in 0..20 {
                if !daemon_is_healthy(port).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        let _ = std::fs::remove_file(&paths.pid_path);
    }

    Ok(stopped_any)
}

pub fn format_pid_entry(pid: u32, port: u16) -> String {
    format!("{pid}:{port}")
}

/// Parses `pid:port` (current format) or bare `pid` (legacy v0.1.2 format, scoped to `DEFAULT_PORT`).
pub fn parse_pid_entry(raw: &str) -> Option<(u32, u16)> {
    let trimmed = raw.trim();
    if let Some((pid_s, port_s)) = trimmed.split_once(':') {
        let pid = pid_s.trim().parse::<u32>().ok()?;
        let port = port_s.trim().parse::<u16>().ok()?;
        Some((pid, port))
    } else {
        let pid = trimmed.parse::<u32>().ok()?;
        Some((pid, DEFAULT_PORT))
    }
}

pub fn read_log_tail(log_path: &Path, lines: usize) -> Result<String> {
    if !log_path.exists() {
        return Ok(String::new());
    }
    let content = std::fs::read_to_string(log_path)?;
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(lines);
    Ok(all[start..].join("\n"))
}

fn signal_pid_terminate(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn current_unix_uid() -> Option<String> {
    let out = Command::new("id").arg("-u").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if uid.is_empty() { None } else { Some(uid) }
}

fn escape_xml(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn pid_entry_scopes_fallback_signal_to_matching_port() {
        assert_eq!(
            parse_pid_entry(&format_pid_entry(4242, 10199)),
            Some((4242, 10199))
        );
        assert_eq!(parse_pid_entry("4242\n"), Some((4242, DEFAULT_PORT)));
        assert_eq!(parse_pid_entry("bad"), None);
    }

    #[test]
    fn macos_autostart_generates_crash_only_keepalive_plist_and_toggles_cleanly() {
        let tmp = tempdir().unwrap();
        let mgr = AutoStartManager::for_testing(
            tmp.path().join("home"),
            PathBuf::from("/Users/test/.0x1-memory/bin/0x1-memory"),
            tmp.path().join("daemon.log"),
            10101,
            AutoStartPlatform::MacOs,
        );
        assert!(mgr.status().supported);
        assert!(!mgr.status().enabled);

        let enabled = mgr.enable().unwrap();
        assert!(enabled.enabled);
        let plist_path = mgr.service_file_path().unwrap();
        let xml = std::fs::read_to_string(&plist_path).unwrap();
        assert!(xml.contains(MACOS_LAUNCHD_LABEL));
        assert!(xml.contains("<string>--foreground</string>"));
        assert!(xml.contains("<string>10101</string>"));
        assert!(xml.contains("<key>SuccessfulExit</key>"));
        assert!(xml.contains("<false/>"));

        let disabled = mgr.disable().unwrap();
        assert!(!disabled.enabled);
        assert!(!plist_path.exists());
    }

    #[test]
    fn linux_autostart_generates_systemd_user_unit_and_windows_reports_unsupported() {
        let tmp = tempdir().unwrap();
        let linux = AutoStartManager::for_testing(
            tmp.path().join("home"),
            PathBuf::from("/home/u/.0x1-memory/bin/0x1-memory"),
            tmp.path().join("daemon.log"),
            10101,
            AutoStartPlatform::Linux,
        );
        assert!(linux.enable().unwrap().enabled);
        let unit = std::fs::read_to_string(linux.service_file_path().unwrap()).unwrap();
        assert!(unit.contains(
            "ExecStart=\"/home/u/.0x1-memory/bin/0x1-memory\" serve --foreground --port 10101"
        ));
        assert!(unit.contains("Restart=on-failure"));
        assert!(!linux.disable().unwrap().enabled);

        let win = AutoStartManager::for_testing(
            tmp.path().join("home"),
            PathBuf::from("C:\\0x1-memory.exe"),
            tmp.path().join("daemon.log"),
            10101,
            AutoStartPlatform::Windows,
        );
        assert!(!win.status().supported);
        assert!(win.enable().is_err());
    }

    #[test]
    fn log_tail_reads_last_lines_without_panicking() {
        let tmp = tempdir().unwrap();
        let log = tmp.path().join("daemon.log");
        assert_eq!(read_log_tail(&log, 10).unwrap(), "");
        std::fs::write(&log, "line 1\nline 2\nline 3\nline 4\n").unwrap();
        assert_eq!(read_log_tail(&log, 2).unwrap(), "line 3\nline 4");
    }
}
