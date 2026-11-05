//! Local working folder.
//!
//! Layout: `<root>/<Project name>/<stable file name>` (e.g. `Suspensão FS-26/manga.dwg`).
//! Files keep the same name across versions so AutoCAD xrefs keep resolving.
//! Files you have not checked out are marked read-only (in projects that
//! require check-out), so nobody edits a file without locking it first.
//!
//! What the client knows about each working copy (which version it came from,
//! its hash) lives in `<root>/.paddock/<server>.json`.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::api::{Item, Version};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalStatus {
    /// Not in the working folder.
    Missing,
    /// Same content as the latest server version.
    UpToDate,
    /// Unchanged locally, but the server has a newer version.
    Outdated,
    /// Edited locally, based on the latest version (ready to check in).
    Modified,
    /// Edited locally AND the server has a newer version.
    Conflict,
    /// A file with this name exists but was not downloaded by Paddock.
    Untracked,
}

impl LocalStatus {
    pub fn key(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::UpToDate => "uptodate",
            Self::Outdated => "outdated",
            Self::Modified => "modified",
            Self::Conflict => "conflict",
            Self::Untracked => "untracked",
        }
    }

    pub fn is_modified(self) -> bool {
        matches!(self, Self::Modified | Self::Conflict | Self::Untracked)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    /// Path relative to the workspace root.
    path: PathBuf,
    /// Server version this working copy is based on.
    version_id: i64,
    label: String,
    /// Hash of the downloaded/published content (the "base").
    base_sha256: String,
    /// Cache of the current file hash, valid while mtime/size are unchanged.
    cached_mtime: u64,
    cached_size: u64,
    cached_sha256: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    server: String,
    entries: HashMap<i64, Entry>,
}

pub struct Workspace {
    root: PathBuf,
    state_path: PathBuf,
    state: State,
}

pub struct LocalInfo {
    pub status: LocalStatus,
    pub path: PathBuf,
    pub base_label: Option<String>,
    pub base_version_id: Option<i64>,
}

impl Workspace {
    pub fn open(root: &Path, server_url: &str) -> Result<Self> {
        let meta = root.join(".paddock");
        fs::create_dir_all(&meta)
            .with_context(|| format!("Não foi possível criar a pasta de trabalho {}", root.display()))?;
        hide_on_windows(&meta);
        let state_path = meta.join(format!("{}.json", sanitize(&server_key(server_url))));
        let state = fs::read(&state_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| State { server: server_url.to_owned(), ..Default::default() });
        Ok(Self { root: root.to_path_buf(), state_path, state })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save(&self) -> Result<()> {
        let tmp = self.state_path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&self.state)?)?;
        fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }

    pub fn project_dir(&self, project_name: &str) -> PathBuf {
        self.root.join(sanitize(project_name))
    }

    pub fn item_path(&self, item: &Item) -> PathBuf {
        self.project_dir(&item.project_name).join(sanitize(&item.file_name))
    }

    /// Compare the working copy with the server's latest version.
    pub fn inspect(&mut self, item: &Item) -> LocalInfo {
        let path = self.item_path(item);
        let latest_id = item.latest_version.as_ref().map(|v| v.id);
        let entry = self.state.entries.get(&item.id).cloned();
        let mut info = LocalInfo {
            status: LocalStatus::Missing,
            path: path.clone(),
            base_label: entry.as_ref().map(|e| e.label.clone()),
            base_version_id: entry.as_ref().map(|e| e.version_id),
        };
        if !path.is_file() {
            return info;
        }
        let Some(entry) = entry.filter(|e| self.root.join(&e.path) == path) else {
            // File present but not downloaded by us. If identical to latest, adopt it.
            if let (Some(v), Ok(sha)) = (&item.latest_version, hash_file(&path)) {
                if sha == v.sha256 {
                    let _ = self.record(item, v, &sha);
                    info.status = LocalStatus::UpToDate;
                    info.base_label = Some(v.label.clone());
                    info.base_version_id = Some(v.id);
                    return info;
                }
            }
            info.status = LocalStatus::Untracked;
            return info;
        };
        let current = match self.current_hash(item.id, &path) {
            Ok(h) => h,
            Err(_) => return info,
        };
        let modified = current != entry.base_sha256;
        let outdated = Some(entry.version_id) != latest_id;
        info.status = match (modified, outdated) {
            (false, false) => LocalStatus::UpToDate,
            (false, true) => LocalStatus::Outdated,
            (true, false) => LocalStatus::Modified,
            (true, true) => LocalStatus::Conflict,
        };
        info
    }

    /// Hash with an mtime/size cache so refreshing a folder of big CAD files is cheap.
    fn current_hash(&mut self, item_id: i64, path: &Path) -> Result<String> {
        let (mtime, size) = stat(path)?;
        if let Some(e) = self.state.entries.get(&item_id) {
            if e.cached_mtime == mtime && e.cached_size == size && !e.cached_sha256.is_empty() {
                return Ok(e.cached_sha256.clone());
            }
        }
        let sha = hash_file(path)?;
        if let Some(e) = self.state.entries.get_mut(&item_id) {
            e.cached_mtime = mtime;
            e.cached_size = size;
            e.cached_sha256 = sha.clone();
        }
        Ok(sha)
    }

    /// Remember that the working copy of `item` now holds `version` (hash `sha`).
    pub fn record(&mut self, item: &Item, version: &Version, sha: &str) -> Result<()> {
        let path = self.item_path(item);
        let (mtime, size) = stat(&path)?;
        let rel = path.strip_prefix(&self.root).unwrap_or(&path).to_path_buf();
        self.state.entries.insert(
            item.id,
            Entry {
                path: rel,
                version_id: version.id,
                label: version.label.clone(),
                base_sha256: sha.to_owned(),
                cached_mtime: mtime,
                cached_size: size,
                cached_sha256: sha.to_owned(),
            },
        );
        self.save()
    }

    /// Move a locally edited file to `.paddock/backup/` so nothing is ever lost.
    pub fn backup(&self, path: &Path) -> Result<PathBuf> {
        let dir = self.root.join(".paddock").join("backup");
        fs::create_dir_all(&dir)?;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("arquivo");
        let ext = path.extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let dest = dir.join(format!("{stem}_{}{ext}", crate::fmt::file_timestamp(ts)));
        set_writable(path, true)?;
        fs::copy(path, &dest)?;
        Ok(dest)
    }
}

// ------------------------------------------------------------------ file helpers

fn server_key(url: &str) -> String {
    url.trim_start_matches("https://").trim_start_matches("http://").to_owned()
}

/// Make a path component valid on Windows and Linux.
pub fn sanitize(name: &str) -> String {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let mut s: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).to_owned();
    if s.is_empty() {
        s = "_".into();
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        s.insert(0, '_');
    }
    s
}

fn stat(path: &Path) -> Result<(u64, u64)> {
    let m = fs::metadata(path)?;
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    Ok((mtime, m.len()))
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Toggle write permission for the owner.
/// On Windows this is the read-only attribute (AutoCAD then opens the file read-only).
/// On Linux and macOS only the owner's write bit changes (std's `set_readonly(false)`
/// would make the file world-writable). AutoCAD for Mac and Finder both honour it.
pub fn set_writable(path: &Path, writable: bool) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut perm = fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perm.mode();
        perm.set_mode(if writable { mode | 0o200 } else { mode & !0o222 });
    }
    #[cfg(not(unix))]
    {
        perm.set_readonly(!writable);
    }
    fs::set_permissions(path, perm).with_context(|| format!("Não foi possível alterar permissões de {}", path.display()))
}

/// Atomically put `src` at `dest`, even if `dest` is read-only (Windows refuses
/// to replace read-only files).
pub fn replace_file(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        set_writable(dest, true)?;
    }
    fs::rename(src, dest).map_err(|e| {
        let _ = fs::remove_file(src);
        if cfg!(windows) && e.raw_os_error() == Some(32) {
            anyhow::anyhow!("O arquivo {} está aberto em outro programa. Feche-o e tente novamente.", dest.display())
        } else {
            anyhow::anyhow!("Não foi possível gravar {}: {e}", dest.display())
        }
    })
}

#[cfg(windows)]
fn hide_on_windows(path: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("attrib")
        .arg("+h")
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(not(windows))]
fn hide_on_windows(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize("Suspensão FS-26"), "Suspensão FS-26");
        assert_eq!(sanitize("a/b:c?"), "a_b_c_");
        assert_eq!(sanitize("CON.dwg"), "_CON.dwg");
        assert_eq!(sanitize("final. "), "final");
    }

    #[test]
    fn writable_toggle_and_replace() {
        let dir = std::env::temp_dir().join(format!("paddock-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.dwg");
        let b = dir.join("b.tmp");
        fs::write(&a, b"one").unwrap();
        set_writable(&a, false).unwrap();
        assert!(fs::metadata(&a).unwrap().permissions().readonly());
        fs::write(&b, b"two").unwrap();
        replace_file(&b, &a).unwrap();
        assert_eq!(fs::read(&a).unwrap(), b"two");
        set_writable(&a, true).unwrap();
        assert!(!fs::metadata(&a).unwrap().permissions().readonly());
        fs::remove_dir_all(&dir).unwrap();
    }
}
