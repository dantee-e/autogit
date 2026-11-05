//! Per-user client settings, stored as JSON in the OS config directory:
//! Windows: %APPDATA%\Paddock\Paddock\config\config.json
//! macOS:   ~/Library/Application Support/br.Paddock.Paddock/config.json
//! Linux:   ~/.config/paddock/config.json

use std::path::PathBuf;

use anyhow::Result;
use directories::{ProjectDirs, UserDirs};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub server_url: String,
    pub username: String,
    /// Session token. Stored in the user's private config dir (0600 on Linux).
    pub token: Option<String>,
    pub workspace: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub open_after_checkout: bool,
}

fn default_true() -> bool {
    true
}

fn dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("br", "Paddock", "Paddock")
}

pub fn config_path() -> PathBuf {
    dirs()
        .map(|d| d.config_dir().join("config.json"))
        .unwrap_or_else(|| PathBuf::from("paddock-config.json"))
}

impl Config {
    pub fn load() -> Self {
        std::fs::read(config_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| Config { open_after_checkout: true, ..Default::default() })
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn workspace_dir(&self) -> PathBuf {
        self.workspace.clone().unwrap_or_else(default_workspace)
    }
}

pub fn default_workspace() -> PathBuf {
    UserDirs::new()
        .and_then(|u| u.document_dir().map(|d| d.to_path_buf()).or_else(|| Some(u.home_dir().to_path_buf())))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Paddock")
}

/// Name for this computer, sent at login so sessions are identifiable.
pub fn device_name() -> String {
    let host = std::env::var("COMPUTERNAME") // Windows
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(mac_computer_name)
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_owned())) // Linux
        .or_else(|| command_output("hostname", &[]))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "computador".into());
    format!("Paddock desktop ({host}, {})", std::env::consts::OS)
}

/// macOS has no /etc/hostname and HOSTNAME is usually unset in GUI apps;
/// the friendly name ("MacBook da Ana") comes from scutil.
#[cfg(target_os = "macos")]
fn mac_computer_name() -> Option<String> {
    command_output("scutil", &["--get", "ComputerName"])
}

#[cfg(not(target_os = "macos"))]
fn mac_computer_name() -> Option<String> {
    None
}

fn command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
